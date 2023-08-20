use anyhow::Result;
use inotify::{Inotify, WatchMask};
use log::*;
use pipeswitch_lib::config::Config;
use pipeswitch_lib::{Pipeswitch, PipeswitchMessage};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

#[derive(Debug)]
pub enum Event {
    Pipeswitch(PipeswitchMessage),
    ConfigModified(Config),
}

pub fn load_config_or_default(path: &Path) -> Result<Config> {
    Ok(if let Some((conf, _)) = Config::load_from(path)? {
        trace!("Found existing config");
        conf
    } else {
        let (conf, doc) = Config::default_conf()?;
        trace!("Writing default config");
        conf.write_to(path, Some(&doc))?;
        conf
    })
}

pub struct ConfigListener {
    running: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}

impl ConfigListener {
    pub fn start(path: &Path, sender: Sender<Event>) -> ConfigListener {
        let running = Arc::new(AtomicBool::new(true));

        let join_handle = std::thread::spawn({
            let running = running.clone();
            let path = path.to_owned();
            move || {
                let mut inotify =
                    Inotify::init().expect("Error while initializing inotify instance");
                inotify
                    .watches().add(&path, WatchMask::MODIFY)
                    .expect("Failed to add file watch");
                while running.load(Ordering::Relaxed) {
                    let mut buffer = [0; 1024];
                    let events = inotify
                        .read_events_blocking(&mut buffer)
                        .expect("Error while reading events");
                    for _ in events {
                        match load_config_or_default(&path) {
                            Ok(cfg) => {
                                sender
                                    .send(Event::ConfigModified(cfg))
                                    .expect("Failed to send ConfigModified");
                            }
                            Err(err) => {
                                error!("Error loading updated config: {err}")
                            }
                        };
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

pub fn start_pipeswitch_thread(sender: Sender<Event>) -> Result<(Pipeswitch, JoinHandle<()>)> {
    let (ps_sender, ps_receiver) = channel();
    let ps = Pipeswitch::new(Some(ps_sender))?;
    Ok((
        ps,
        std::thread::spawn(move || {
            while let Ok(msg) = ps_receiver.recv() {
                sender.send(Event::Pipeswitch(msg)).unwrap();
            }
        }),
    ))
}
