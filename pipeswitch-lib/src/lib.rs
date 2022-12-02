pub use log;
use pipewire::channel::Sender as PipewireSender;
pub use pipewire::types::ObjectType;
use pw::{
    mainloop::{mainloop, MainloopAction, MainloopEvents},
    types::{Link, Object, Port},
};
pub use pw::{types, PipewireError, PipewireState};
use std::{
    marker::PhantomData,
    sync::{
        mpsc::{self},
        Arc, Mutex, MutexGuard,
    },
    thread::JoinHandle,
};
use thiserror::Error;
pub use toml_edit;

pub mod config;
mod pw;

#[derive(Error, Debug)]
pub enum PipeswitchError {
    #[error("error reading or writing to disk: {0}")]
    IOError(#[from] std::io::Error),
    #[error("error with PipeWire interface: {0}")]
    PipewireError(#[from] pw::PipewireError),
    #[error("error converting config: {0}")]
    TomlConvertError(#[from] toml_edit::TomlError),
    #[error("error serializing config: {0}")]
    TomlSerializationError(#[from] toml_edit::ser::Error),
    #[error("error parsing config: {0}")]
    TomlDeserializationError(#[from] toml_edit::de::Error),
    #[error("no Link Factory found!")]
    NoLinkFactory,
    #[error("failure in background thread: {0}")]
    CriticalThreadFailure(&'static str),
    #[error("given ports are both input: {0:?}, {1:?}")]
    DoubleInputPort(Port, Port),
    #[error("given ports are both output: {0:?}, {1:?}")]
    DoubleOutputPort(Port, Port),
    #[cfg(debug_assertions)]
    #[error("unknown error")]
    Unknown,
}

#[derive(Debug)]
pub enum PipeswitchMessage {
    NewObject(Object),
    ObjectRemoved(Object),
    Error(pw::PipewireError),
}

pub struct Pipeswitch {
    pipewire_state: Arc<Mutex<PipewireState>>,
    sender: PipewireSender<MainloopAction>,
    mainloop_receiver: mpsc::Receiver<MainloopEvents>,
    join_handle: Option<JoinHandle<()>>,
    nosync_phantom_data: PhantomData<std::cell::Cell<()>>,
}

impl Pipeswitch {
    pub fn new(sender: Option<mpsc::Sender<PipeswitchMessage>>) -> Result<Self, PipeswitchError> {
        let pipewire_state = Arc::new(Mutex::new(PipewireState::default()));

        let (ps_sender, ps_receiver) = mpsc::channel();
        let (pw_sender, pw_receiver) = pipewire::channel::channel::<MainloopAction>();

        let state_clone = pipewire_state.clone();

        let join_handle = std::thread::spawn(move || {
            mainloop(sender, ps_sender, pw_receiver, state_clone.clone())
                .map_err(|_| {
                    PipeswitchError::CriticalThreadFailure("Background thread died unexpectedly")
                })
                .unwrap();
        });

        Ok(Pipeswitch {
            pipewire_state,
            sender: pw_sender,
            join_handle: Some(join_handle),
            mainloop_receiver: ps_receiver,
            nosync_phantom_data: PhantomData::default(),
        })
    }

    pub fn lock_current_state(&self) -> MutexGuard<PipewireState> {
        self.pipewire_state.lock().unwrap()
    }

    pub fn create_link(
        &self,
        port1: Port,
        port2: Port,
        rule_name: String,
    ) -> Result<Option<Link>, PipeswitchError> {
        use types::Direction::*;
        // Check for double inputs and double outputs
        match (&port1.direction, &port2.direction) {
            (Input, Input) => return Err(PipeswitchError::DoubleInputPort(port1, port2)),
            (Output, Output) => return Err(PipeswitchError::DoubleOutputPort(port1, port2)),
            _ => {}
        };
        // Flip ports if necessary
        let (input, output) = if let Input = &port1.direction {
            (port1, port2)
        } else {
            (port2, port1)
        };

        let lock = self.pipewire_state.lock().unwrap();
        let factory_name = lock
            .factories
            .get("PipeWire:Interface:Link")
            .ok_or(PipeswitchError::NoLinkFactory)?
            .name
            .clone();
        drop(lock);

        self.sender
            .send(MainloopAction::CreateLink(
                factory_name,
                output,
                input,
                rule_name,
            ))
            .map_err(|_| PipeswitchError::CriticalThreadFailure("Failed to send create link"))
            .unwrap();

        let link = loop {
            if let Ok(MainloopEvents::LinkCreated(link)) = self.mainloop_receiver.recv() {
                break link;
            }
        };
        Ok(link)
    }
}

impl Drop for Pipeswitch {
    fn drop(&mut self) {
        let _ = self.sender.send(MainloopAction::Terminate);
        if let Some(handle) = self.join_handle.take() {
            handle
                .join()
                .map_err(|_| {
                    PipeswitchError::CriticalThreadFailure("Failed to wait thread to stop")
                })
                .unwrap();
        }
    }
}
