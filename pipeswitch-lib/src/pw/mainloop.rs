use super::PipewireMessage;
use crate::{
    types::{self, Object},
    PipeswitchMessage, PipewireError, PipewireState,
};
use pipewire::{
    channel::Receiver as PipewireReceiver,
    link as pwlink,
    proxy::ProxyT,
    registry::{GlobalObject, Registry},
    spa::{AsyncSeq, ForeignDict},
    types::ObjectType,
    Context, Core, MainLoop, PW_ID_CORE,
};
use std::{
    collections::HashMap,
    sync::{
        mpsc::{self, Sender},
        Arc, Mutex,
    },
};

#[derive(Debug)]
pub enum MainloopAction {
    Terminate,
    CreateLink(String, types::Port, types::Port, String),
}

#[derive(Debug)]
pub enum MainloopEvents {
    LinkCreated(Option<types::Link>),
}

struct Roundtrip(AsyncSeq, u32);

type ShareableMainloopData = Arc<Mutex<MainloopData>>;

struct MainloopData {
    mainloop: MainLoop,
    core: Core,
    pending_seq: Option<Roundtrip>,
    link_info: HashMap<u32, types::Link>,
    link_listeners: HashMap<u32, (pwlink::Link, pwlink::LinkListener)>,
    event_sender: Sender<MainloopEvents>,
    message_sender: Option<Sender<PipeswitchMessage>>,
}

impl MainloopData {
    fn from(
        mainloop: MainLoop,
        core: Core,
        event_sender: Sender<MainloopEvents>,
        message_sender: Option<Sender<PipeswitchMessage>>,
    ) -> Self {
        MainloopData {
            mainloop,
            core,
            event_sender,
            message_sender,
            pending_seq: None,
            link_info: HashMap::default(),
            link_listeners: HashMap::default(),
        }
    }
}

pub fn mainloop(
    sender: Option<Sender<PipeswitchMessage>>,
    ps_sender: mpsc::Sender<MainloopEvents>,
    receiver: PipewireReceiver<MainloopAction>,
    state: Arc<Mutex<PipewireState>>,
) -> Result<(), PipewireError> {
    let mainloop = MainLoop::new()?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;

    let registry = Arc::new(core.get_registry()?);

    let data = Arc::new(Mutex::new(MainloopData::from(
        mainloop.clone(),
        core.clone(),
        ps_sender.clone(),
        sender.clone(),
    )));

    let _rec = receiver.attach(&mainloop, {
        let data = data.clone();
        // Called when Pipeswitch sends an event
        move |action| handle_action(action, &data)
    });
    let _listener_core = core
        .add_listener_local()
        .done({
            let data = data.clone();
            // Called when Core is done with roundtrip
            move |id, seq| handle_done(id, seq, &data)
        })
        .register();
    let _listener = registry
        .add_listener_local()
        .global({
            let state = state.clone();
            let data = data.clone();
            let registry = registry.clone();
            move |global| handle_new_global(global, &data, &registry, &state)
        })
        .global_remove({
            let state = state.clone();
            let data = data.clone();
            move |global_id| {
                process_message(PipewireMessage::GlobalRemoved(global_id), &data, &state)
            }
        })
        .register();

    mainloop.run();

    Ok(())
}

/// Called when an action is called from the Pipeswitch-struct
fn handle_action(action: MainloopAction, data: &ShareableMainloopData) {
    match action {
        MainloopAction::Terminate => data.lock().unwrap().mainloop.quit(),
        MainloopAction::CreateLink(factory_name, output, input, rule_name) => {
            let props = pipewire::properties! {
                *pipewire::keys::LINK_OUTPUT_NODE => output.node_id.to_string(),
                *pipewire::keys::LINK_OUTPUT_PORT => output.id.to_string(),
                *pipewire::keys::LINK_INPUT_NODE => input.node_id.to_string(),
                *pipewire::keys::LINK_INPUT_PORT => input.id.to_string(),
                "object.linger" => "1",
                types::KEY_RULE_NAME => rule_name
            };
            let mut data_lock = data.lock().unwrap();
            let proxy = data_lock
                .core
                .create_object::<pipewire::link::Link, _>(&factory_name, &props)
                .unwrap();
            let proxy_id = proxy.upcast_ref().id();

            if let Some(info) = data_lock.link_info.get(&proxy_id) {
                data_lock
                    .event_sender
                    .send(MainloopEvents::LinkCreated(Some(info.clone())))
                    .unwrap();
            } else {
                let listener = proxy
                    .add_listener_local()
                    .info({
                        let data = data.clone();
                        move |info| {
                            data.lock()
                                .unwrap()
                                .link_info
                                .insert(proxy_id, types::Link::from_link_info(info).unwrap());
                        }
                    })
                    .register();
                data_lock.link_listeners.insert(proxy_id, (proxy, listener));
                data_lock.pending_seq = Some(Roundtrip(
                    data_lock.core.sync(0).expect("sync failed"),
                    proxy_id,
                ));
            }
        }
    }
}

/// Called when a round trip is complete from the Core
fn handle_done(id: u32, seq: AsyncSeq, data: &ShareableMainloopData) {
    let mut data_lock = data.lock().unwrap();
    if id == PW_ID_CORE {
        match data_lock.pending_seq {
            Some(Roundtrip(s, id)) => {
                if s == seq {
                    data_lock.link_listeners.remove(&id);
                    let link = data_lock.link_info.remove(&id);
                    data_lock
                        .event_sender
                        .send(MainloopEvents::LinkCreated(link))
                        .unwrap();
                    data_lock.pending_seq = None;
                }
            }
            None => {}
        }
    }
}

fn handle_new_global(
    global: &GlobalObject<ForeignDict>,
    data: &ShareableMainloopData,
    registry: &Registry,
    state: &Arc<Mutex<PipewireState>>,
) {
    match global.type_ {
        ObjectType::Link => {
            let proxy: pipewire::link::Link = registry.bind(&global).unwrap();
            let proxy_id = proxy.upcast_ref().id();
            let listener = proxy
                .add_listener_local()
                .info({
                    let data = data.clone();
                    let state = state.clone();
                    move |info| {
                        process_message(
                            PipewireMessage::NewGlobal(
                                info.id(),
                                ObjectType::Link,
                                Object::Link(types::Link::from_link_info(info).unwrap()),
                            ),
                            &data,
                            &state,
                        );
                        let mut data_lock = data.lock().unwrap();
                        data_lock.link_listeners.remove(&proxy_id);
                    }
                })
                .register();
            let mut data_lock = data.lock().unwrap();
            data_lock.link_listeners.insert(proxy_id, (proxy, listener));
        }
        _ => match Object::from_global(&global) {
            Ok(Some(obj)) => {
                process_message(
                    PipewireMessage::NewGlobal(global.id, global.type_.clone(), obj),
                    &data,
                    &state,
                );
            }
            Err(e) => {
                let data_lock = data.lock().unwrap();
                if let Some(sender) = &data_lock.message_sender {
                    sender.send(PipeswitchMessage::Error(e)).unwrap();
                }
            }
            _ => {}
        },
    }
}

fn process_message(
    new_global: PipewireMessage,
    data: &ShareableMainloopData,
    state: &Arc<Mutex<PipewireState>>,
) {
    let data_lock = data.lock().unwrap();
    let result = state.lock().unwrap().process_message(new_global);
    if let (Some(sender), Some(result)) = (&data_lock.message_sender, result) {
        sender.send(PipeswitchMessage::NewObject(result)).unwrap();
    }
}
