use std::{
    sync::{mpsc::Sender, Arc, Mutex, MutexGuard},
    thread::JoinHandle,
};

use pipewire::channel::Sender as PipewireSender;
pub use pw::PipewireError;
use pw::{mainloop, types::PipewireObject, PipewireState, Terminate};

mod pw;

pub enum PipeswitchMessage {
    NewObject(PipewireObject),
    ObjectRemoved(PipewireObject),
}

pub struct Pipeswitch {
    pipewire_state: Arc<Mutex<PipewireState>>,
    sender: PipewireSender<Terminate>,
    join_handle: Option<JoinHandle<()>>,
}

impl Pipeswitch {
    pub fn new(sender: Option<Sender<PipeswitchMessage>>) -> Result<Self, PipewireError> {
        let pipewire_state = Arc::new(Mutex::new(PipewireState::default()));

        let (pw_sender, pw_receiver) = pipewire::channel::channel::<Terminate>();

        let state_clone = pipewire_state.clone();

        let join_handle =
            std::thread::spawn(move || mainloop(sender, pw_receiver, state_clone.clone()).unwrap());

        Ok(Pipeswitch {
            pipewire_state,
            sender: pw_sender,
            join_handle: Some(join_handle),
        })
    }

    pub fn lock_current_state(&self) -> MutexGuard<PipewireState> {
        self.pipewire_state.lock().unwrap()
    }
}

impl Drop for Pipeswitch {
    fn drop(&mut self) {
        let _ = self.sender.send(Terminate);
        if let Some(handle) = self.join_handle.take() {
            handle.join().unwrap()
        }
    }
}
