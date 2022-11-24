use std::{
    sync::{
        mpsc::{self},
        Arc, Mutex, MutexGuard,
    },
    thread::JoinHandle,
};

use pipewire::channel::Sender as PipewireSender;
pub use pw::PipewireError;
use pw::{mainloop, types::PipewireObject, MainloopActions, MainloopEvents, PipewireState};

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

    pub fn roundtrip(&self) -> Result<(), PipewireError> {
        self.sender
            .send(MainloopActions::Roundtrip)
            .map_err(|_| PipewireError::Unknown)?;
        loop {
            if let Ok(MainloopEvents::Done) = self.mainloop_receiver.recv() {
                break;
            }
        }
        Ok(())
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
