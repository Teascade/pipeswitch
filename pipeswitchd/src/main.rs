use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::mpsc::channel,
};

use config::{load_config_or_default, start_pipeswitch_thread, ConfigListener};
use log::*;
use pipeswitch_lib::{
    config::{Config, NodeOrTarget},
    types::{PipewireObject, Port},
    Pipeswitch, PipeswitchMessage, PipewireState,
};
use regex::Regex;

use crate::config::Event;

mod config;

#[derive(Debug)]
struct Rule {
    name: String,
    client: Option<Regex>,
    node: Option<Regex>,
    port: Option<Regex>,
    matching_ports: HashSet<u32>,
}

impl Rule {
    fn from_node_or_target(name: String, node_or_target: &NodeOrTarget) -> Rule {
        match node_or_target {
            NodeOrTarget::NodeName(node_name) => Rule {
                name,
                client: None,
                node: Some(Regex::from_str(&node_name).unwrap()),
                port: None,
                matching_ports: HashSet::new(),
            },
            NodeOrTarget::Target(t) => Rule {
                name,
                client: t.client.as_ref().map(|rex| Regex::from_str(&rex).unwrap()),
                node: t.node.as_ref().map(|rex| Regex::from_str(&rex).unwrap()),
                port: t.port.as_ref().map(|rex| Regex::from_str(&rex).unwrap()),
                matching_ports: HashSet::new(),
            },
        }
    }
}
fn matches_entirely(regex: &Regex, text: &str) -> Option<bool> {
    let first_match = regex.captures(text)?.get(0)?;
    Some(first_match.start() == 0 && first_match.end() == text.len())
}

impl Rule {
    fn add_if_matches(&mut self, port: &Port, state: &PipewireState) -> bool {
        let port_matches = match &self.port {
            Some(regex) => matches_entirely(&regex, &port.name).unwrap_or(false),
            _ => true,
        };

        if port_matches {
            let node = state.nodes.get(&port.node_id);
            let client = node.map(|n| state.clients.get(&n.client_id)).flatten();

            let node_matches = match (&self.node, node) {
                (Some(regex), Some(node)) => {
                    matches_entirely(&regex, &node.node_name).unwrap_or(false)
                }
                (Some(_), None) => false,
                _ => true,
            };
            let client_matches = match (&self.client, client) {
                (Some(regex), Some(client)) => {
                    matches_entirely(&regex, &client.application_name).unwrap_or(false)
                }
                (Some(_), None) => false,
                _ => true,
            };

            if node_matches && client_matches {
                self.matching_ports.insert(port.id);
                let alias = &port.alias;
                let direction = &port.direction;
                let name = &self.name;
                debug!("[{name}].{direction:?} + {alias}");
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    fn delete(&mut self, port: &Port) -> bool {
        let was = self.matching_ports.remove(&port.id);
        if was {
            let alias = &port.alias;
            let direction = &port.direction;
            let name = &self.name;
            debug!("[{name}].{direction:?} - {alias}");
        }
        was
    }
}

#[derive(Debug)]
struct LinkRules {
    input: Rule,
    output: Rule,
    special_empty_ports: bool,
}

impl LinkRules {
    fn should_connect(&self, port1: &Port, port2: &Port) -> bool {
        let ports_some = self.input.port.is_some() || self.output.port.is_some();
        let same_channel = port2.channel == port1.channel;
        !self.special_empty_ports || ports_some || same_channel
    }
}

struct PipeswitchDaemon {
    rules: HashMap<String, LinkRules>,
    pipeswitch: Pipeswitch,
}

impl PipeswitchDaemon {
    pub fn new(pipeswitch: Pipeswitch) -> Self {
        PipeswitchDaemon {
            pipeswitch,
            rules: HashMap::default(),
        }
    }

    fn update_config(&mut self, config: &Config) {
        debug!("Updating config");
        for (name, link) in &config.links {
            let rules = LinkRules {
                input: Rule::from_node_or_target(name.clone(), &link.input),
                output: Rule::from_node_or_target(name.clone(), &link.output),
                special_empty_ports: link.special_empty_ports,
            };
            debug!("New link: {name}");
            debug!("{rules:?}");
            self.rules.insert(name.clone(), rules);
        }
    }

    fn new_port(&mut self, port: &Port) {
        use pipeswitch_lib::types::Direction;
        let state = self.pipeswitch.lock_current_state();
        match port.direction {
            Direction::Input => {
                for rule in self.rules.values_mut() {
                    if rule.input.add_if_matches(&port, &state) {
                        for out_port_id in &rule.output.matching_ports {
                            let old_port = state.ports.get(&out_port_id).unwrap();
                            if rule.should_connect(&port, &old_port) {
                                let np_name = &port.alias;
                                let op_name = &old_port.alias;
                                info!("{np_name} should connect to {op_name}")
                            }
                        }
                    }
                }
            }
            Direction::Output => {
                for rule in self.rules.values_mut() {
                    if rule.output.add_if_matches(&port, &state) {
                        for out_port_id in &rule.input.matching_ports {
                            let old_port = state.ports.get(&out_port_id).unwrap();
                            if rule.should_connect(&port, &old_port) {
                                let np_name = &port.alias;
                                let op_name = &old_port.alias;
                                info!("{np_name} should connect to {op_name}")
                            }
                        }
                    }
                }
            }
        }
    }

    fn port_deleted(&mut self, port: &Port) {
        use pipeswitch_lib::types::Direction;
        match &port.direction {
            Direction::Input => {
                for rule in self.rules.values_mut() {
                    rule.input.delete(port);
                }
            }
            Direction::Output => {
                for rule in self.rules.values_mut() {
                    rule.output.delete(port);
                }
            }
        }
    }
}

fn main() {
    let config_path = &Config::default_path().unwrap();
    let config = load_config_or_default(&config_path);

    stderrlog::new()
        .module(module_path!())
        .verbosity(config.log.level)
        .init()
        .unwrap();
    let (sender, receiver) = channel();

    let (pipeswitch, _join) = start_pipeswitch_thread(sender.clone());
    let mut daemon = PipeswitchDaemon::new(pipeswitch);
    daemon.update_config(&config);

    let _listener = ConfigListener::start(&config_path, sender.clone());
    while let Ok(event) = receiver.recv() {
        match event {
            Event::Pipeswitch(pw) => {
                use PipeswitchMessage::*;
                match pw {
                    NewObject(PipewireObject::Port(port)) => daemon.new_port(&port),
                    ObjectRemoved(PipewireObject::Port(port)) => daemon.port_deleted(&port),
                    Error(e) => {
                        warn!("{e}")
                    }
                    _ => (),
                }
            }
            Event::ConfigModified(conf) => {
                daemon.update_config(&conf);
                trace!("Shutting down..");
                break;
            }
        }
    }

    // while let Ok(keycode) = receiver.recv() {
    //     println!("starting");
    //     let state = ps.lock_current_state();
    //     println!("Current nodes:");
    //     let mut output = None;
    //     let mut input = None;
    //     let mut name = "spotify";
    //     if keycode == Keycode::A {
    //         name = "Firefox"
    //     }
    //     for (node_id, node) in &state.nodes {
    //         let node_name = &node.node_name;
    //         let ports = state.ports_by_node(*node_id);
    //         let names: Vec<&String> = state
    //             .ports_by_node(*node_id)
    //             .into_iter()
    //             .map(|p| &p.name)
    //             .collect();
    //         println!("Node {node_id} '{node_name}' with ports {names:?}");
    //         if node.node_name == "WEBRTC VoiceEngine" {
    //             for port in &ports {
    //                 if port.name == "input_MONO" {
    //                     input = Some((*port).clone());
    //                 }
    //             }
    //         }
    //         if node.node_name == name {
    //             for port in &ports {
    //                 if port.name == "output_FR" {
    //                     output = Some((*port).clone());
    //                 }
    //             }
    //         }
    //     }
    //     drop(state);

    //     if let (Some(o), Some(i)) = (output, input) {
    //         let link = ps.create_link(o, i).unwrap();
    //         println!("{link:?}");
    //     }

    //     if keycode == Keycode::Escape {
    //         break;
    //     }
    // }
}
