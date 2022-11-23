use std::{
    num::ParseIntError,
    str::ParseBoolError,
    sync::mpsc::{self, Sender},
};

use pipewire::{
    channel::Receiver,
    keys::*,
    registry::GlobalObject,
    spa::{ForeignDict, ReadableDict},
    types::ObjectType,
    Context, MainLoop,
};

use thiserror::Error;
pub use tokio;

struct Terminate;

const VERSION: u32 = 3;

// const PORT_PATH: &str = "object.path"; // Firefox:output_0 / Firefox:output_1
// const PORT_SERIAL: &str = "object.serial"; // 2252 / 2253
// const PORT_ID: &str = "port.id"; // 0 / 1
// const PORT_DSP: &str = "format.dsp"; // 0 / 1
// const PORT_NAME: &str = "port.name"; // output_FL / output_FR
// const PORT_DIRECTION: &str = "port.direction"; // out
// const PORT_ALIAS: &str = "port.alias"; // Firefox:output_FL / Firefox:output_FR
// const NODE_ID: &str = "node.id"; // 788
// const AUDIO_CHANNEL: &str = "audio.channel";
// const PORT_PHYSICAL: &str = "port.physical";
// const PORT_TERMINAL: &str = "port.terminal";

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
}

#[derive(Debug)]
struct Port {
    // TODO: Re-evaluate bit-depth?
    id: u32,
    // TODO: Re-evaluate bit-depth?
    port_id: u32,
    path: Option<String>,
    // TODO: Re-evaluate bit-depth?
    node_id: u32,
    dsp: Option<String>,
    channel: Option<String>,
    name: String,
    direction: String,
    alias: String,
    physical: Option<bool>,
    terminal: Option<bool>,
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
    sender: Sender<PipewireMessage>,
    receiver: Receiver<Terminate>,
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
            let s = sender.clone();
            move |global| match PipewireObject::from_global(global) {
                Ok(Some(obj)) => {
                    s.send(PipewireMessage::NewGlobal(global.id, obj)).unwrap();
                }
                Err(e) => println!("{e} {global:?}"),
                _ => {}
            }
        })
        .global_remove(move |global_id| {
            sender
                .send(PipewireMessage::GlobalRemoved(global_id))
                .unwrap();
        })
        .register();

    // Calling the `destroy_global` method on the registry will destroy the object with the specified id on the remote.
    // We don't have a specific object to destroy now, so this is commented out.
    // registry.destroy_global(313).into_result()?;

    mainloop.run();

    Ok(())
}

pub fn create_mainloop() -> Result<(), String> {
    // This is running on a core thread.

    let (main_sender, main_receiver) = mpsc::channel();
    let (pw_sender, pw_receiver) = pipewire::channel::channel();

    std::thread::spawn(move || mainloop(main_sender, pw_receiver).unwrap());

    loop {
        let message = main_receiver.recv().unwrap();
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
