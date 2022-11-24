use pipewire::{channel::Receiver as PipewireReceiver, types::ObjectType, Context, MainLoop};
use std::{
    collections::HashMap,
    num::ParseIntError,
    str::ParseBoolError,
    sync::{mpsc::Sender, Arc, Mutex},
};
use thiserror::Error;

mod types;

use types::VERSION;

use self::types::{Client, Link, Node, PipewireObject, Port};

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
    NewGlobal(u32, ObjectType, PipewireObject),
    GlobalRemoved(u32),
}

#[derive(Default)]
pub struct PipewireState {
    pub object_types: HashMap<u32, ObjectType>,
    pub ports: HashMap<u32, Port>,
    pub nodes: HashMap<u32, Node>,
    pub links: HashMap<u32, Link>,
    pub clients: HashMap<u32, Client>,
}

impl PipewireState {
    fn process_message(&mut self, message: PipewireMessage) {
        match message {
            PipewireMessage::NewGlobal(id, obj_type, object) => {
                self.object_types.insert(id, obj_type);
                match object {
                    PipewireObject::Port(port) => drop(self.ports.insert(id, port)),
                    PipewireObject::Node(node) => drop(self.nodes.insert(node.id, node)),
                    PipewireObject::Link(link) => drop(self.links.insert(link.id, link)),
                    PipewireObject::Client(client) => drop(self.clients.insert(client.id, client)),
                }
            }
            PipewireMessage::GlobalRemoved(id) => {
                if let Some(obj_type) = self.object_types.get(&id) {
                    match obj_type {
                        ObjectType::Port => drop(self.ports.remove(&id)),
                        ObjectType::Node => drop(self.nodes.remove(&id)),
                        ObjectType::Link => drop(self.links.remove(&id)),
                        ObjectType::Client => drop(self.clients.remove(&id)),
                        _ => {}
                    }
                }
            }
        }
    }

    pub fn ports_by_node(&self, node: &Node) -> Vec<&Port> {
        let mut vec = Vec::new();
        for (_, port) in &self.ports {
            if port.node_id == node.id {
                vec.push(port.clone());
            }
        }
        vec
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
                        .process_message(PipewireMessage::NewGlobal(
                            global.id,
                            global.type_.clone(),
                            obj,
                        ));
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
