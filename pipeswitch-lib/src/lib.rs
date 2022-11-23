use std::{
    num::ParseIntError,
    str::ParseBoolError,
    sync::{mpsc::Sender, Arc, Mutex},
    thread::JoinHandle,
};

use pipewire::{
    channel::{Receiver as PipewireReceiver, Sender as PipewireSender},
    keys::*,
    registry::GlobalObject,
    spa::{ForeignDict, ReadableDict},
    types::ObjectType,
    Context, MainLoop,
};

use thiserror::Error;

struct Terminate;

const VERSION: u32 = 3;

#[derive(Error, Debug)]
pub enum PipeswitchError {
    #[error("Failed to parse int: {0}")]
    ParseIntError(#[from] ParseIntError),
    #[error("Failed to parse boolean: {0}")]
    ParseBoolError(#[from] ParseBoolError),
    #[error("property not found: {0}")]
    PropNotFound(&'static str),
    #[error("object version invalid, expected {VERSION}, got {0}")]
    InvalidVersion(u32),
    #[error("GlobalObject does not have properties")]
    MissingProps,
    #[cfg(debug_assertions)]
    #[error("unknown error")]
    Unknown,
}

// TODO: Re-evaluate bit-depth?
type PipewireNum = u32;

#[derive(Debug)]
pub struct Port {
    pub id: PipewireNum,
    pub port_id: PipewireNum,
    pub path: Option<String>,
    pub node_id: PipewireNum,
    pub dsp: Option<String>,
    pub channel: Option<String>,
    pub name: String,
    pub direction: String,
    pub alias: String,
    pub physical: Option<bool>,
    pub terminal: Option<bool>,
}

impl Port {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Self, PipeswitchError> {
        use PipeswitchError::*;
        if global.version != VERSION {
            Err(InvalidVersion(global.version))?
        }
        let props = global.props.as_ref().ok_or(MissingProps)?;
        let get_prop = |property| props.get(property).ok_or(PropNotFound(property));
        Ok(Port {
            id: global.id,
            port_id: get_prop(*PORT_ID)?.parse()?,
            path: props.get(*OBJECT_PATH).map(|v| v.to_string()),
            node_id: get_prop(*NODE_ID)?.parse()?,
            dsp: props.get(*FORMAT_DSP).map(|v| v.to_string()),
            channel: props.get(*AUDIO_CHANNEL).map(|v| v.to_string()),
            name: get_prop(*PORT_NAME)?.to_string(),
            direction: get_prop(*PORT_DIRECTION)?.to_string(),
            alias: get_prop(*PORT_ALIAS)?.to_string(),
            physical: props.get(*PORT_PHYSICAL).map(|v| v.parse()).transpose()?,
            terminal: props.get(*PORT_TERMINAL).map(|v| v.parse()).transpose()?,
        })
    }
}

enum PipewireObject {
    Port(Port),
}

impl PipewireObject {
    fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Option<Self>, PipeswitchError> {
        match global.type_ {
            ObjectType::Port => Ok(Some(Self::Port(Port::from_global(global)?))),
            _ => Ok(None),
        }
    }
}

enum PipewireMessage {
    NewGlobal(u32, PipewireObject),
    GlobalRemoved(u32),
}

fn mainloop(
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
                Err(e) => println!("{e} {global:?}"),
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

struct PipewireState {}

impl PipewireState {
    fn process_message(&mut self, message: PipewireMessage) {
        match message {
            PipewireMessage::NewGlobal(id, PipewireObject::Port(port)) => {
                println!("+ Port {id} {port:?}")
            }
            PipewireMessage::GlobalRemoved(id) => {
                println!("- Something {id}")
            }
        }
    }
}

pub struct Pipeswitch {
    pipewire_state: Arc<Mutex<PipewireState>>,
    sender: PipewireSender<Terminate>,
    join_handle: Option<JoinHandle<()>>,
}

impl Pipeswitch {
    pub fn new(sender: Option<Sender<()>>) -> Result<Self, PipeswitchError> {
        let pipewire_state = Arc::new(Mutex::new(PipewireState {}));

        let (pw_sender, pw_receiver) = pipewire::channel::channel::<Terminate>();

        let state_clone = pipewire_state.clone();

        let join_handle =
            std::thread::spawn(move || mainloop(sender, pw_receiver, state_clone.clone()).unwrap());

        Ok(Pipeswitch {
            pipewire_state,
            sender: pw_sender,
            join_handle: Some(join_handle),
        })
    }

    pub fn shutdown(self) {}
}

impl Drop for Pipeswitch {
    fn drop(&mut self) {
        let _ = self.sender.send(Terminate);
        if let Some(handle) = self.join_handle.take() {
            handle.join().unwrap()
        }
    }
}
