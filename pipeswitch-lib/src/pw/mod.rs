use pipewire::{channel::Receiver as PipewireReceiver, Context, MainLoop};
use std::{
    num::ParseIntError,
    str::ParseBoolError,
    sync::{mpsc::Sender, Arc, Mutex},
};
use thiserror::Error;

mod types;

use types::VERSION;

use self::types::PipewireObject;

#[derive(Error, Debug)]
pub enum PipewireError {
    #[error("Failed to parse int: {0}")]
    ParseIntError(#[from] ParseIntError),
    #[error("Failed to parse boolean: {0}")]
    ParseBoolError(#[from] ParseBoolError),
    #[error("({0}) property not found: {1}")]
    PropNotFound(&'static str, &'static str),
    #[error("object version invalid, expected {VERSION}, got {0}")]
    InvalidVersion(u32),
    #[error("globalobject does not have properties: {0}")]
    MissingProps(u32),
    #[error("direction not valid: {0}")]
    InvalidDirection(String),
    #[cfg(debug_assertions)]
    #[error("unknown error")]
    Unknown,
}

enum PipewireMessage {
    NewGlobal(u32, PipewireObject),
    GlobalRemoved(u32),
}

pub(crate) struct PipewireState {}

impl PipewireState {
    fn process_message(&mut self, message: PipewireMessage) {
        match message {
            PipewireMessage::NewGlobal(id, object) => match object {
                PipewireObject::Port(port) => println!("+ Port {id} {port:?}"),
                PipewireObject::Node(node) => println!("+ Node {id} {node:?}"),
                PipewireObject::Link(link) => println!("+ Link {id} {link:?}"),
                PipewireObject::Client(client) => println!("+ Client {id} {client:?}"),
            },
            PipewireMessage::GlobalRemoved(id) => {
                println!("- Something {id}")
            }
        }
    }
}

pub(crate) struct Terminate;

pub(crate) fn mainloop(
    _: Option<Sender<()>>,
    receiver: PipewireReceiver<Terminate>,
    state: Arc<Mutex<PipewireState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mainloop = MainLoop::new()?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let registry = core.get_registry()?;

    let _rec = receiver.attach(&mainloop, {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    // This needs to be a listener for doing roundtrips to server, later.
    let _listener_core = core.add_listener_local().done(|_, _| ());

    let _listener = registry
        .add_listener_local()
        .global({
            let state = state.clone();
            move |global| match PipewireObject::from_global(global) {
                Ok(Some(obj)) => {
                    state
                        .lock()
                        .unwrap()
                        .process_message(PipewireMessage::NewGlobal(global.id, obj));
                }
                Err(e) => println!("{e}\n    {global:?}"),
                _ => {}
            }
        })
        .global_remove(move |global_id| {
            state
                .lock()
                .unwrap()
                .process_message(PipewireMessage::GlobalRemoved(global_id));
        })
        .register();

    mainloop.run();

    Ok(())
}
