use pipewire::types::ObjectType;
use std::{collections::HashMap, num::ParseIntError, str::ParseBoolError};
use thiserror::Error;

pub(crate) mod mainloop;
pub mod types;
use types::VERSION;

use self::types::{Client, Factory, Link, Node, Object, Port};

#[derive(Error, Debug)]
pub enum PipewireError {
    #[error("Failed to parse int: {0}")]
    ParseIntError(#[from] ParseIntError),
    #[error("Failed to parse boolean: {0}")]
    ParseBoolError(#[from] ParseBoolError),
    #[error("property '{3}' not found in object {0} of type {1}: {2:?}")]
    PropNotFound(u32, ObjectType, HashMap<String, String>, &'static str),
    #[error("object version invalid, expected {VERSION}, got {0}")]
    InvalidVersion(u32),
    #[error("globalobject does not have properties: {1} ({0}) {2:?}")]
    MissingProps(u32, ObjectType, HashMap<String, String>),
    #[error("direction not valid: {0}")]
    InvalidDirection(String),
    #[error("direction not valid: {0}")]
    InvalidChannel(String),
    #[error("error with core pipewire interface: {0}")]
    PipewireInterfaceError(#[from] pipewire::Error),
    #[cfg(debug_assertions)]
    #[error("unknown error")]
    Unknown,
}

#[derive(Debug)]
enum PipewireMessage {
    NewGlobal(u32, ObjectType, Object),
    GlobalRemoved(u32),
}

#[derive(Debug, Default)]
pub struct PipewireState {
    pub object_types: HashMap<u32, ObjectType>,
    pub ports: HashMap<u32, Port>,
    pub nodes: HashMap<u32, Node>,
    pub links: HashMap<u32, Link>,
    pub clients: HashMap<u32, Client>,
    pub factories: HashMap<String, Factory>,
}

impl PipewireState {
    fn process_message(&mut self, message: PipewireMessage) -> Option<Object> {
        match message {
            PipewireMessage::NewGlobal(id, obj_type, object) => {
                self.object_types.insert(id, obj_type);
                match object.clone() {
                    Object::Port(port) => drop(self.ports.insert(id, port)),
                    Object::Node(node) => drop(self.nodes.insert(node.id, node)),
                    Object::Link(link) => {
                        drop(self.links.insert(link.id, link));
                    }
                    Object::Client(client) => drop(self.clients.insert(client.id, client)),
                    Object::Factory(factory) => {
                        drop(self.factories.insert(factory.type_name.clone(), factory))
                    }
                }
                Some(object)
            }
            PipewireMessage::GlobalRemoved(id) => {
                if let Some(obj_type) = self.object_types.get(&id) {
                    match obj_type {
                        ObjectType::Port => self.ports.remove(&id).map(|p| Object::Port(p)),
                        ObjectType::Node => self.nodes.remove(&id).map(|n| Object::Node(n)),
                        ObjectType::Link => self.links.remove(&id).map(|l| Object::Link(l)),
                        ObjectType::Client => self.clients.remove(&id).map(|c| Object::Client(c)),
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
