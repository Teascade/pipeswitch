use inotify::{Inotify, WatchMask};
use log::*;
use pipeswitch_lib::config::Config;
use pipeswitch_lib::{Pipeswitch, PipeswitchMessage};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

#[derive(Debug)]
pub enum Event {
    Pipeswitch(PipeswitchMessage),
    ConfigModified(Config),
}

pub fn load_config_or_default(path: &PathBuf) -> Config {
    if let Some((conf, _)) = Config::load_from(path).unwrap() {
        trace!("Found existing config");
        conf
    } else {
        let (conf, doc) = Config::default_conf().unwrap();
        trace!("Writing default config");
        conf.write_to(path, Some(&doc)).unwrap();
        conf
    }
}

pub struct ConfigListener {
    running: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}

impl ConfigListener {
    pub fn start(path: &PathBuf, sender: Sender<Event>) -> ConfigListener {
        let running = Arc::new(AtomicBool::new(true));

        let join_handle = std::thread::spawn({
            let running = running.clone();
            let path = path.clone();
            move || {
                let mut inotify =
                    Inotify::init().expect("Error while initializing inotify instance");
                inotify
                    .add_watch(&path, WatchMask::MODIFY)
                    .expect("Failed to add file watch");
                while running.load(Ordering::Relaxed) {
                    let mut buffer = [0; 1024];
                    let events = inotify
                        .read_events_blocking(&mut buffer)
                        .expect("Error while reading events");
                    for _ in events {
                        sender
                            .send(Event::ConfigModified(load_config_or_default(&path)))
                            .unwrap();
                    }
                }
            }
        });
        ConfigListener {
            join_handle: Some(join_handle),
            running,
        }
    }
}

impl Drop for ConfigListener {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.join_handle.take().unwrap().join().unwrap()
    }
}

pub fn start_pipeswitch_thread(sender: Sender<Event>) -> (Pipeswitch, JoinHandle<()>) {
    let (ps_sender, ps_receiver) = channel();
    let ps = Pipeswitch::new(Some(ps_sender)).unwrap();
    (
        ps,
        std::thread::spawn(move || {
            while let Ok(msg) = ps_receiver.recv() {
                sender.send(Event::Pipeswitch(msg)).unwrap();
            }
        }),
    )
}
