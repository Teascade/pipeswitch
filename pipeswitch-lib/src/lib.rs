use std::{
    sync::{
        mpsc::{self},
        Arc, Mutex, MutexGuard,
    },
    thread::JoinHandle,
};

use pipewire::channel::Sender as PipewireSender;
pub use pw::PipewireError;
use pw::{
    mainloop,
    types::{Link, PipewireObject, Port},
    MainloopActions, MainloopEvents, PipewireState,
};

pub mod config;
mod pw;

pub enum PipeswitchMessage {
    NewObject(PipewireObject),
    ObjectRemoved(PipewireObject),
}

pub struct Pipeswitch {
    pipewire_state: Arc<Mutex<PipewireState>>,
    sender: PipewireSender<MainloopActions>,
    mainloop_receiver: mpsc::Receiver<MainloopEvents>,
    join_handle: Option<JoinHandle<()>>,
}

impl Pipeswitch {
    pub fn new(sender: Option<mpsc::Sender<PipeswitchMessage>>) -> Result<Self, PipewireError> {
        let pipewire_state = Arc::new(Mutex::new(PipewireState::default()));

        let (ps_sender, ps_receiver) = mpsc::channel();
        let (pw_sender, pw_receiver) = pipewire::channel::channel::<MainloopActions>();

        let state_clone = pipewire_state.clone();

        let join_handle = std::thread::spawn(move || {
            mainloop(sender, ps_sender, pw_receiver, state_clone.clone()).unwrap()
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

    pub fn create_link(&self, output: Port, input: Port) -> Result<Option<Link>, PipewireError> {
        let lock = self.pipewire_state.lock().unwrap();
        let factory_name = lock
            .factories
            .get("PipeWire:Interface:Link")
            .ok_or(PipewireError::Unknown)?
            .name
            .clone();
        drop(lock);

        self.sender
            .send(MainloopActions::CreateLink(factory_name, output, input))
            .map_err(|_| PipewireError::Unknown)?;

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
            handle.join().unwrap()
        }
    }
}
