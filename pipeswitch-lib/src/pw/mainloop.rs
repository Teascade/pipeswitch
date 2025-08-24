use super::PipewireMessage;
use crate::{
    types::{self, map_props, Object},
    PipeswitchMessage, PipewireError, PipewireState,
};
use pipewire::{
    channel::Receiver as PipewireReceiver,
    context::Context,
    core::Core,
    link::{self as pwlink},
    main_loop::MainLoop,
    proxy::ProxyT,
    registry::{GlobalObject, Registry},
    spa::utils::{dict::DictRef, result::AsyncSeq},
    types::ObjectType, // Context, Core, MainLoop, PW_ID_CORE,
};
use std::{
    collections::HashMap,
    sync::{
        mpsc::{self, Sender},
        Arc, Mutex,
    },
};

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum MainloopAction {
    Terminate,
    CreateLink(String, types::Port, types::Port, String),
    DestroyLink(types::Link),
}

#[derive(Debug)]
pub enum MainloopEvents {
    LinkCreated(Option<types::Link>),
    LinkDestroyed(bool),
}

enum Roundtrip {
    CreateLink(AsyncSeq, u32),
    DestroyLink(AsyncSeq),
}

type ShareableMainloopData = Arc<Mutex<MainloopData>>;

struct LinkProxy {
    _proxy: pwlink::Link,
    link: Option<types::Link>,
    listener: Option<pwlink::LinkListener>,
}

struct MainloopData {
    mainloop: MainLoop,
    core: Core,
    pending_seq: Option<Roundtrip>,
    links: HashMap<u32, LinkProxy>,
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
            links: HashMap::default(),
        }
    }
}

pub fn mainloop(
    sender: Option<Sender<PipeswitchMessage>>,
    ps_sender: mpsc::Sender<MainloopEvents>,
    receiver: PipewireReceiver<MainloopAction>,
    state: Arc<Mutex<PipewireState>>,
) -> Result<(), PipewireError> {
    let mainloop = MainLoop::new(None)?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let registry = Arc::new(core.get_registry()?);

    let data = Arc::new(Mutex::new(MainloopData::from(
        mainloop.clone(),
        core.clone(),
        ps_sender,
        sender,
    )));

    let _rec = receiver.attach(&mainloop.loop_(), {
        let data = data.clone();
        let registry = registry.clone();
        // Called when Pipeswitch sends an event
        move |action| handle_action(action, &data, &registry)
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
            move |global_id| {
                process_message(PipewireMessage::GlobalRemoved(global_id), &data, &state)
            }
        })
        .register();

    mainloop.run();

    Ok(())
}

/// Called when an action is called from the Pipeswitch-struct
fn handle_action(action: MainloopAction, data: &ShareableMainloopData, registry: &Registry) {
    match action {
        MainloopAction::Terminate => data.lock().unwrap().mainloop.quit(),
        MainloopAction::CreateLink(factory_name, output, input, rule_name) => {
            let props = pipewire::properties::properties! {
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
                .create_object::<pipewire::link::Link>(&factory_name, &props)
                .unwrap();
            let proxy_id = proxy.upcast_ref().id();

            if let Some(info) = data_lock.links.get(&proxy_id).and_then(|l| l.link.clone()) {
                data_lock
                    .event_sender
                    .send(MainloopEvents::LinkCreated(Some(info)))
                    .unwrap();
            } else {
                let listener = proxy
                    .add_listener_local()
                    .info({
                        let data = data.clone();
                        move |info| {
                            if let Some(link_proxy) = data.lock().unwrap().links.get_mut(&proxy_id)
                            {
                                link_proxy.link =
                                    Some(types::Link::from_link_info(info, proxy_id).unwrap())
                            }
                        }
                    })
                    .register();
                data_lock.links.insert(
                    proxy_id,
                    LinkProxy {
                        _proxy: proxy,
                        link: None,
                        listener: Some(listener),
                    },
                );
                data_lock.pending_seq = Some(Roundtrip::CreateLink(
                    data_lock.core.sync(0).expect("sync failed"),
                    proxy_id,
                ));
            }
        }
        MainloopAction::DestroyLink(link) => {
            let mut data_lock = data.lock().unwrap();
            if let Some(proxy) = data_lock.links.remove(&link.proxy_id) {
                if proxy.link.is_some() || proxy.listener.is_some() {
                    data_lock
                        .event_sender
                        .send(MainloopEvents::LinkDestroyed(false))
                        .unwrap();
                    data_lock.links.insert(link.proxy_id, proxy);
                } else {
                    registry.destroy_global(link.id);
                    data_lock.pending_seq = Some(Roundtrip::DestroyLink(
                        data_lock.core.sync(0).expect("sync failed"),
                    ));
                }
            }
        }
    }
}

/// Called when a round trip is complete from the Core
fn handle_done(id: u32, seq: AsyncSeq, data: &ShareableMainloopData) {
    let mut data_lock = data.lock().unwrap();
    if id == pipewire::core::PW_ID_CORE {
        match data_lock.pending_seq {
            Some(Roundtrip::CreateLink(s, id)) => {
                if s == seq {
                    if let Some(proxy) = data_lock.links.get_mut(&id) {
                        let _listener = proxy.listener.take();
                        let link = proxy.link.take();
                        data_lock
                            .event_sender
                            .send(MainloopEvents::LinkCreated(link))
                            .unwrap();
                        data_lock.pending_seq = None;
                    }
                }
            }
            Some(Roundtrip::DestroyLink(s)) => {
                if s == seq {
                    data_lock
                        .event_sender
                        .send(MainloopEvents::LinkDestroyed(true))
                        .unwrap();
                    data_lock.pending_seq = None;
                }
            }
            None => {}
        }
    }
}

fn handle_new_global(
    global: &GlobalObject<&DictRef>,
    data: &ShareableMainloopData,
    registry: &Registry,
    state: &Arc<Mutex<PipewireState>>,
) {
    match global.type_ {
        ObjectType::Link => {
            let proxy: pipewire::link::Link = registry.bind(global).unwrap();
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
                                Object::Link(types::Link::from_link_info(info, proxy_id).unwrap()),
                            ),
                            &data,
                            &state,
                        );
                        let mut data_lock = data.lock().unwrap();
                        if let Some(proxy) = data_lock.links.get_mut(&proxy_id) {
                            proxy.listener.take();
                        }
                    }
                })
                .register();
            let mut data_lock = data.lock().unwrap();
            data_lock.links.insert(
                proxy_id,
                LinkProxy {
                    _proxy: proxy,
                    link: None,
                    listener: Some(listener),
                },
            );
        }
        _ => match Object::from_global(global) {
            Ok(Some(obj)) => {
                process_message(
                    PipewireMessage::NewGlobal(global.id, global.type_.clone(), obj),
                    data,
                    state,
                );
            }
            Err(e) => {
                let data_lock = data.lock().unwrap();
                if let Some(sender) = &data_lock.message_sender {
                    sender.send(PipeswitchMessage::Error(e)).unwrap();
                    // TODO: Error here!
                }
            }
            _ => {}
        },
    }
}

fn process_message(
    message: PipewireMessage,
    data: &ShareableMainloopData,
    state: &Arc<Mutex<PipewireState>>,
) {
    let data_lock = data.lock().unwrap();
    let result = state.lock().unwrap().process_message(message);
    if let (Some(sender), Some(result)) = (&data_lock.message_sender, result) {
        sender.send(result).unwrap();
    }
}
