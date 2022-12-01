pub use log;
use pipewire::channel::Sender as PipewireSender;
pub use pipewire::types::ObjectType;
use pw::{
    mainloop,
    types::{Link, PipewireObject, Port},
    MainloopActions, MainloopEvents, PipewireState,
};
pub use pw::{types, PipewireError};
use std::{
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
    #[cfg(debug_assertions)]
    #[error("unknown error")]
    Unknown,
}

#[derive(Debug)]
pub enum PipeswitchMessage {
    NewObject(PipewireObject),
    ObjectRemoved(PipewireObject),
    Error(pw::PipewireError),
}

pub struct Pipeswitch {
    pipewire_state: Arc<Mutex<PipewireState>>,
    sender: PipewireSender<MainloopActions>,
    mainloop_receiver: mpsc::Receiver<MainloopEvents>,
    join_handle: Option<JoinHandle<()>>,
}

impl Pipeswitch {
    pub fn new(sender: Option<mpsc::Sender<PipeswitchMessage>>) -> Result<Self, PipeswitchError> {
        let pipewire_state = Arc::new(Mutex::new(PipewireState::default()));

        let (ps_sender, ps_receiver) = mpsc::channel();
        let (pw_sender, pw_receiver) = pipewire::channel::channel::<MainloopActions>();

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
        })
    }

    pub fn lock_current_state(&self) -> MutexGuard<PipewireState> {
        self.pipewire_state.lock().unwrap()
    }

    pub fn create_link(&self, output: Port, input: Port) -> Result<Option<Link>, PipeswitchError> {
        let lock = self.pipewire_state.lock().unwrap();
        let factory_name = lock
            .factories
            .get("PipeWire:Interface:Link")
            .ok_or(PipeswitchError::NoLinkFactory)?
            .name
            .clone();
        drop(lock);

        self.sender
            .send(MainloopActions::CreateLink(factory_name, output, input))
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
        let _ = self.sender.send(MainloopActions::Terminate);
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
