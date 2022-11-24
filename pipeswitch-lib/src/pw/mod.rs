use pipewire::{
    channel::Receiver as PipewireReceiver, types::ObjectType, Context, MainLoop, PW_ID_CORE,
};
use std::{
    cell::Cell,
    collections::HashMap,
    num::ParseIntError,
    rc::Rc,
    str::ParseBoolError,
    sync::{
        mpsc::{self, Sender},
        Arc, Mutex,
    },
};
use thiserror::Error;

pub mod types;

use types::VERSION;

use crate::PipeswitchMessage;

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
    fn process_message(&mut self, message: PipewireMessage) -> Option<PipewireObject> {
        use PipewireObject::*;
        match message {
            PipewireMessage::NewGlobal(id, obj_type, object) => {
                self.object_types.insert(id, obj_type);
                match object.clone() {
                    PipewireObject::Port(port) => drop(self.ports.insert(id, port)),
                    PipewireObject::Node(node) => drop(self.nodes.insert(node.id, node)),
                    PipewireObject::Link(link) => drop(self.links.insert(link.id, link)),
                    PipewireObject::Client(client) => drop(self.clients.insert(client.id, client)),
                }
                Some(object)
            }
            PipewireMessage::GlobalRemoved(id) => {
                if let Some(obj_type) = self.object_types.get(&id) {
                    match obj_type {
                        ObjectType::Port => self.ports.remove(&id).map(|p| Port(p)),
                        ObjectType::Node => self.nodes.remove(&id).map(|n| Node(n)),
                        ObjectType::Link => self.links.remove(&id).map(|l| Link(l)),
                        ObjectType::Client => self.clients.remove(&id).map(|c| Client(c)),
                        _ => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    pub fn ports_by_node(&self, node_id: u32) -> Vec<&Port> {
        let mut vec = Vec::new();
        for (_, port) in &self.ports {
            if port.node_id == node_id {
                vec.push(port);
            }
        }
        vec
    }
}

pub(crate) enum MainloopActions {
    Terminate,
    Roundtrip,
}

pub(crate) enum MainloopEvents {
    Done,
}

pub(crate) fn mainloop(
    sender: Option<Sender<PipeswitchMessage>>,
    ps_sender: mpsc::Sender<MainloopEvents>,
    receiver: PipewireReceiver<MainloopActions>,
    state: Arc<Mutex<PipewireState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mainloop = MainLoop::new()?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let registry = core.get_registry()?;

    let pending_seq = Rc::new(Cell::new(None));

    let _rec = receiver.attach(&mainloop, {
        let mainloop = mainloop.clone();
        let core = core.clone();
        let pending_seq = pending_seq.clone();
        move |action| match action {
            MainloopActions::Terminate => mainloop.quit(),
            MainloopActions::Roundtrip => {
                if pending_seq.get().is_none() {
                    pending_seq.set(Some(core.sync(0).expect("sync failed")));
                }
            }
        }
    });

    // This needs to be a listener for doing roundtrips to server, later.
    let _listener_core = core
        .add_listener_local()
        .done({
            move |id, seq| {
                let pending = pending_seq.get();
                if pending.is_some() {
                    if id == PW_ID_CORE && Some(seq) == pending {
                        ps_sender.send(MainloopEvents::Done).unwrap();
                        pending_seq.set(None);
                    }
                }
            }
        })
        .register();

    let _listener = registry
        .add_listener_local()
        .global({
            let state = state.clone();
            let sender = sender.clone();
            move |global| match PipewireObject::from_global(global) {
                Ok(Some(obj)) => {
                    let result = state
                        .lock()
                        .unwrap()
                        .process_message(PipewireMessage::NewGlobal(
                            global.id,
                            global.type_.clone(),
                            obj,
                        ));
                    if let (Some(sender), Some(result)) = (&sender, result) {
                        sender.send(PipeswitchMessage::NewObject(result)).unwrap()
                    }
                }
                Err(e) => println!("{e}\n    {global:?}"),
                _ => {}
            }
        })
        .global_remove(move |global_id| {
            let result = state
                .lock()
                .unwrap()
                .process_message(PipewireMessage::GlobalRemoved(global_id));
            if let (Some(sender), Some(result)) = (&sender, result) {
                sender
                    .send(PipeswitchMessage::ObjectRemoved(result))
                    .unwrap()
            }
        })
        .register();

    mainloop.run();

    Ok(())
}
