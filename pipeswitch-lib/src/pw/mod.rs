use pipewire::{
    channel::Receiver as PipewireReceiver, proxy::ProxyT, spa::AsyncSeq, types::ObjectType,
    Context, MainLoop, PW_ID_CORE,
};
use std::{
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

use self::types::{Client, Factory, Link, Node, PipewireObject, Port};

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
    pub factories: HashMap<String, Factory>,
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
                    PipewireObject::Link(link) => {
                        drop(self.links.insert(link.id, link));
                    }
                    PipewireObject::Client(client) => drop(self.clients.insert(client.id, client)),
                    PipewireObject::Factory(factory) => {
                        drop(self.factories.insert(factory.type_name.clone(), factory))
                    }
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
    CreateLink(String, Port, Port),
}

#[derive(Debug)]
pub(crate) enum MainloopEvents {
    LinkCreated(Option<Link>),
}

enum Roundtrip {
    Internal(AsyncSeq, u32),
}

pub(crate) fn mainloop(
    sender: Option<Sender<PipeswitchMessage>>,
    ps_sender: mpsc::Sender<MainloopEvents>,
    receiver: PipewireReceiver<MainloopActions>,
    state: Arc<Mutex<PipewireState>>,
) -> Result<(), PipewireError> {
    let mainloop = MainLoop::new()?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let registry = core.get_registry()?;

    let pending_seq = Rc::new(Mutex::new(None));
    let link_info: Rc<Mutex<HashMap<u32, Link>>> = Rc::new(Mutex::new(HashMap::new()));
    let proxy_map = Rc::new(Mutex::new(HashMap::new()));

    let _rec = receiver.attach(&mainloop, {
        let mainloop = mainloop.clone();
        let core = core.clone();
        let pending_seq = pending_seq.clone();
        let ps_sender = ps_sender.clone();
        let link_info = link_info.clone();
        let proxies = proxy_map.clone();
        move |action| match action {
            MainloopActions::Terminate => mainloop.quit(),
            MainloopActions::CreateLink(factory_name, output, input) => {
                let props = pipewire::properties! {
                    *pipewire::keys::LINK_OUTPUT_NODE => output.node_id.to_string(),
                    *pipewire::keys::LINK_OUTPUT_PORT => output.id.to_string(),
                    *pipewire::keys::LINK_INPUT_NODE => input.node_id.to_string(),
                    *pipewire::keys::LINK_INPUT_PORT => input.id.to_string(),
                    "object.linger" => "1"
                };
                let proxy = core
                    .create_object::<pipewire::link::Link, _>(&factory_name, &props)
                    .unwrap();
                let proxy_id = proxy.upcast_ref().id();

                let info_lock = link_info.lock().unwrap();
                if let Some(info) = info_lock.get(&proxy_id) {
                    ps_sender
                        .send(MainloopEvents::LinkCreated(Some(info.clone())))
                        .unwrap();
                } else {
                    let listener = proxy
                        .add_listener_local()
                        .info({
                            let link_info = link_info.clone();
                            move |info| {
                                let mut info_lock = link_info.lock().unwrap();
                                info_lock.insert(
                                    proxy_id,
                                    Link::from_props(info.id(), info.props()).unwrap(),
                                );
                            }
                        })
                        .register();
                    let mut proxy_lock = proxies.lock().unwrap();
                    proxy_lock.insert(proxy_id, (proxy, listener));
                    let mut lock = pending_seq.lock().unwrap();
                    *lock = Some(Roundtrip::Internal(
                        core.sync(0).expect("sync failed"),
                        proxy_id,
                    ));
                }
            }
        }
    });

    let _listener_core = core
        .add_listener_local()
        .done({
            let ps_sender = ps_sender.clone();
            let proxy_info = link_info.clone();
            let info_listeners = proxy_map.clone();
            move |id, seq| {
                let mut lock = pending_seq.lock().unwrap();
                if id == PW_ID_CORE {
                    match lock.as_ref() {
                        Some(Roundtrip::Internal(s, id)) => {
                            if s == &seq {
                                let mut info_lock = proxy_info.lock().unwrap();
                                info_listeners.lock().unwrap().remove(&id);
                                ps_sender
                                    .send(MainloopEvents::LinkCreated(info_lock.remove(&id)))
                                    .unwrap();
                                *lock = None;
                            }
                        }
                        None => {}
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
                        sender.send(PipeswitchMessage::NewObject(result)).unwrap();
                    }
                }
                Err(e) => {
                    if let Some(sender) = &sender {
                        sender.send(PipeswitchMessage::Error(e)).unwrap();
                    }
                }
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
