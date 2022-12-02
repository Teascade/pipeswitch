use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::channel,
};

use config::{load_config_or_default, start_pipeswitch_thread, ConfigListener};
use log::*;
use pipeswitch_lib::{
    config::{Config, LinkConfig, NodeOrTarget},
    types::{Link, Object, Port},
    Pipeswitch, PipeswitchMessage, PipewireState,
};
use regex::{Regex, RegexBuilder};

use crate::config::Event;

mod config;

#[derive(Debug)]
struct Rule {
    name: String,
    client: Option<Regex>,
    node: Option<Regex>,
    port: Option<Regex>,
    matching_ports: HashSet<u32>,
    special_empty_ports: bool,
    original_config: NodeOrTarget,
}

impl PartialEq for Rule {
    fn eq(&self, other: &Self) -> bool {
        self.original_config == other.original_config
    }
}

impl Rule {
    fn from_node_or_target(name: String, special: bool, node_or_target: &NodeOrTarget) -> Rule {
        match node_or_target {
            NodeOrTarget::NodeName(node_name) => Rule {
                name,
                client: None,
                node: Some(
                    RegexBuilder::new(&node_name)
                        .case_insensitive(true)
                        .build()
                        .unwrap(),
                ),
                port: None,
                matching_ports: HashSet::new(),
                special_empty_ports: special,
                original_config: node_or_target.clone(),
            },
            NodeOrTarget::Target(t) => Rule {
                name,
                client: t.client.as_ref().map(|rex| {
                    RegexBuilder::new(&rex)
                        .case_insensitive(true)
                        .build()
                        .unwrap()
                }),
                node: t.node.as_ref().map(|rex| {
                    RegexBuilder::new(&rex)
                        .case_insensitive(true)
                        .build()
                        .unwrap()
                }),
                port: t.port.as_ref().map(|rex| {
                    RegexBuilder::new(&rex)
                        .case_insensitive(true)
                        .build()
                        .unwrap()
                }),
                matching_ports: HashSet::new(),
                special_empty_ports: special,
                original_config: node_or_target.clone(),
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

    fn ignore_channel(&self, other: &Rule) -> bool {
        let ports_some = self.port.is_some() || other.port.is_some();
        !self.special_empty_ports || ports_some
    }
}

#[derive(Debug)]
struct LinkRules {
    name: String,
    input: Rule,
    output: Rule,
    links: HashSet<u32>,
}

impl From<(String, LinkConfig)> for LinkRules {
    fn from((name, cfg): (String, LinkConfig)) -> Self {
        LinkRules {
            name: name.clone(),
            input: Rule::from_node_or_target(name.clone(), cfg.special_empty_ports, &cfg.input),
            output: Rule::from_node_or_target(name.clone(), cfg.special_empty_ports, &cfg.output),
            links: HashSet::new(),
        }
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

    fn new_link(&mut self, link: Link) {
        if let Some(new_rule_name) = link.rule_name.clone() {
            let mut exists = false;
            for (rule_name, rule) in self.rules.iter_mut() {
                if new_rule_name == *rule_name {
                    rule.links.insert(link.id);
                    let link_id = link.id;
                    trace!("Link {link_id} for rule [{rule_name}]");
                    exists = true;
                }
            }
            if !exists {
                let link_id = link.id;
                if self.pipeswitch.destroy_link(link).unwrap() {
                    warn!("old link {link_id} from old config rule [{new_rule_name}] destroyed");
                }
            }
        }
    }

    fn fetch_links<'a, T: IntoIterator<Item = &'a u32>>(&self, link_ids: T) -> Vec<Link> {
        let mut links = Vec::new();
        for link_id in link_ids.into_iter() {
            let state = self.pipeswitch.lock_current_state();
            if let Some(link) = state.links.get(&link_id) {
                links.push(link.clone());
            }
        }
        links
    }

    fn update_config(&mut self, config: &Config) {
        info!("Updating config..");

        // Contains all of the rule names that still need to be checked.
        let mut dirty_rule_names: HashSet<String> = self
            .rules
            .keys()
            .chain(config.links.keys())
            .cloned()
            .collect();

        // Go through all new and old rules and destroy any links that do not
        // match up with the new configuration.
        for rule_name in dirty_rule_names.clone() {
            let curr_rule = self.rules.get(&rule_name);
            let new_rule = config
                .links
                .get(&rule_name)
                .map(|c| LinkRules::from((rule_name.clone(), c.clone())));

            match (curr_rule, new_rule) {
                (Some(curr), Some(new)) => {
                    if new.input != curr.input || new.output != curr.output {
                        debug!("rule [{rule_name}] changed, removing links temporarily");
                        let mut links = Vec::new();
                        for link_id in &curr.links {
                            let state = self.pipeswitch.lock_current_state();
                            if let Some(link) = state.links.get(link_id) {
                                links.push(link.clone());
                            }
                        }
                        for link in self.fetch_links(&curr.links) {
                            let link_id = link.id;
                            if self.pipeswitch.destroy_link(link).unwrap() {
                                trace!(
                                    "old rule [{rule_name}] link {link_id} temporarily destroyed"
                                );
                            }
                        }
                        self.rules.insert(rule_name, new);
                    } else {
                        debug!("rule [{rule_name}] was unmodified");
                        dirty_rule_names.remove(&rule_name);
                    }
                }
                (Some(curr), None) => {
                    for link in self.fetch_links(&curr.links) {
                        let link_id = link.id;
                        if self.pipeswitch.destroy_link(link).unwrap() {
                            trace!("old rule [{rule_name}] link {link_id} destroyed");
                        }
                    }
                    dirty_rule_names.remove(&rule_name);
                }
                (None, Some(new)) => {
                    self.rules.insert(rule_name, new);
                }
                _ => {}
            }
        }

        let ports: Vec<Port> = self
            .pipeswitch
            .lock_current_state()
            .ports
            .values()
            .cloned()
            .collect();

        // Goes through all the rule_names that still need to have their ports checked
        for port in ports {
            self.new_port_for_rules(port, dirty_rule_names.clone());
        }
    }

    fn new_port(&mut self, port: Port) {
        self.new_port_for_rules(port, self.rules.keys().cloned().collect())
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

    fn link_deleted(&mut self, link: &Link) {
        for rule in self.rules.values_mut() {
            let id = link.id;
            if rule.links.remove(&id) {
                let rule_name = &rule.name;
                trace!("Link {id} from rule [{rule_name}] deleted");
            }
        }
    }

    fn new_port_for_rules(&mut self, port: Port, rules: HashSet<String>) {
        use pipeswitch_lib::types::Direction;
        let mut state = self.pipeswitch.lock_current_state();
        for (_, rule) in self.rules.iter_mut().filter(|(n, _)| rules.contains(*n)) {
            let (r1, r2) = if let Direction::Input = port.direction {
                (&mut rule.input, &mut rule.output)
            } else {
                (&mut rule.output, &mut rule.input)
            };
            if r1.add_if_matches(&port, &state) {
                for old_port_id in &r2.matching_ports {
                    let old_port = state.ports.get(old_port_id).unwrap().clone();
                    if r1.ignore_channel(&r2) || port.channel == old_port.channel {
                        let np_name = &port.alias;
                        let op_name = &old_port.alias;
                        info!("Connecting {np_name} to {op_name}");
                        drop(state);
                        self.pipeswitch
                            .create_link(port.clone(), old_port, rule.name.clone())
                            .unwrap();
                        state = self.pipeswitch.lock_current_state();
                    }
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
                    NewObject(Object::Port(port)) => daemon.new_port(port),
                    NewObject(Object::Link(link)) => daemon.new_link(link),
                    ObjectRemoved(Object::Port(port)) => daemon.port_deleted(&port),
                    ObjectRemoved(Object::Link(link)) => daemon.link_deleted(&link),
                    Error(e) => {
                        warn!("{e}")
                    }
                    _ => (),
                }
            }
            Event::ConfigModified(conf) => {
                daemon.update_config(&conf);
                dbg!("hello??");
            }
        }
    }
}
