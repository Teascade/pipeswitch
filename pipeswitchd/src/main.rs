use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::channel,
};

use config::{load_config_or_default, start_pipeswitch_thread, ConfigListener};
use log::*;
use pipeswitch_lib::{
    config::Config,
    types::{Link, Object, Port},
    Pipeswitch, PipeswitchMessage, PipewireError,
};

use crate::config::Event;
use crate::rules::*;

mod config;
mod rules;

fn main() {
    let config_path = &Config::default_path().unwrap();
    let config = load_config_or_default(config_path)
        .map_err(|e| panic!("Failed to load Config at startup: {e}"))
        .unwrap();

    stderrlog::new()
        .module(module_path!())
        .verbosity(config.log.level)
        .timestamp(stderrlog::Timestamp::Second)
        .init()
        .unwrap();
    let (sender, receiver) = channel();

    let (pipeswitch, _join) = start_pipeswitch_thread(sender.clone())
        .map_err(|e| panic!("Failed to start listening to Pipewire: {e}"))
        .unwrap();
    let mut daemon = PipeswitchDaemon::new(pipeswitch, &config);

    let mut _listener = None;
    if config.general.hotreload_config {
        _listener = Some(ConfigListener::start(config_path, sender));
    }

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
                        if let PipewireError::PropNotFound(..) = e {
                            warn!("{e}")
                        } else {
                            error!("{e}")
                        }
                    }
                    _ => (),
                }
            }
            Event::ConfigModified(conf) => {
                daemon.update_config(&conf);
            }
        }
    }
}

struct PipeswitchDaemon {
    rules: HashMap<String, LinkRules>,
    pipeswitch: Pipeswitch,
    linger_links: bool,
}

impl PipeswitchDaemon {
    pub fn new(pipeswitch: Pipeswitch, config: &Config) -> Self {
        let mut daemon = PipeswitchDaemon {
            pipeswitch,
            rules: HashMap::default(),
            linger_links: false,
        };
        daemon.update_config(config);
        daemon
    }

    fn new_link(&mut self, link: Link) {
        if let Some(new_rule_name) = link.rule_name.clone() {
            let mut exists = false;
            for (rule_name, rule) in self.rules.iter_mut() {
                if new_rule_name == *rule_name
                    && rule.input.matching_ports.contains(&link.input_port)
                    && rule.output.matching_ports.contains(&link.output_port)
                {
                    rule.links.insert(link.id);
                    let link_id = link.id;
                    trace!("New link {link_id} for rule [{rule_name}]");
                    exists = true;
                }
            }
            if !exists {
                let link_id = link.id;
                if !self.linger_links && self.pipeswitch.destroy_link(link).unwrap() {
                    info!("old link {link_id} from old config rule [{new_rule_name}] destroyed");
                }
            }
        }
    }

    fn fetch_links<'a, T: IntoIterator<Item = &'a u32>>(&self, link_ids: T) -> Vec<Link> {
        let mut links = Vec::new();
        for link_id in link_ids.into_iter() {
            let state = self.pipeswitch.lock_current_state();
            if let Some(link) = state.links.get(link_id) {
                links.push(link.clone());
            }
        }
        links
    }

    fn update_config(&mut self, config: &Config) {
        debug!("rechecking config");
        let linger_changed = self.linger_links != config.general.linger_links;
        self.linger_links = config.general.linger_links;

        // Contains all of the rule names that still need to be checked.
        let mut dirty_rule_names: HashSet<String> = self
            .rules
            .keys()
            .chain(config.links.keys())
            .cloned()
            .collect();

        let mut modified_count = 0;
        let mut new_count = 0;
        let mut removed_count = 0;
        let mut lingering_links = 0;

        // Go through all new and old rules and destroy any links that do not
        // match up with the new configuration.
        for rule_name in dirty_rule_names.clone() {
            let curr_rule = self.rules.get(&rule_name);
            let new_rule = config
                .links
                .get(&rule_name)
                .map(|c| LinkRules::from((rule_name.clone(), c.clone())));

            match (curr_rule, new_rule) {
                (Some(curr), Some(mut new)) => {
                    if new.input != curr.input || new.output != curr.output {
                        // Same rule exists, but has changed
                        debug!("rule [{rule_name}] changed");
                        if self.linger_links {
                            new.links.extend(&curr.links);
                        } else {
                            for link in self.fetch_links(&curr.links) {
                                let link_id = link.id;
                                if self.pipeswitch.destroy_link(link).unwrap() {
                                    info!("old rule [{rule_name}] link {link_id} destroyed");
                                }
                            }
                        }
                        self.rules.insert(rule_name, new);
                        modified_count += 1;
                    } else {
                        // Same rule exists and has not changed
                        // Check for lingering links anyway
                        if linger_changed && !self.linger_links {
                            info!("deleting old lingered links");
                            for link in self.fetch_links(&curr.links) {
                                let link_id = link.id;
                                if (!curr.input.matching_ports.contains(&link.input_port)
                                    || !curr.output.matching_ports.contains(&link.output_port))
                                    && self.pipeswitch.destroy_link(link).unwrap()
                                {
                                    info!("old rule [{rule_name}] link {link_id} destroyed");
                                    lingering_links += 1;
                                }
                            }
                        }
                        debug!("rule [{rule_name}] was unmodified");
                        dirty_rule_names.remove(&rule_name);
                    }
                }
                (Some(curr), None) => {
                    // Previous config had a rule which this one does not.
                    for link in self.fetch_links(&curr.links) {
                        let link_id = link.id;
                        if !self.linger_links && self.pipeswitch.destroy_link(link).unwrap() {
                            info!("old rule [{rule_name}] link {link_id} destroyed");
                        }
                    }
                    dirty_rule_names.remove(&rule_name);
                    removed_count += 1;
                }
                (None, Some(new)) => {
                    // New config has a rule previous config did not.
                    self.rules.insert(rule_name, new);
                    new_count += 1;
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
        if !dirty_rule_names.is_empty() {
            trace!("re-checking following rules: {dirty_rule_names:?}");
            for port in ports {
                self.new_port_for_rules(port, dirty_rule_names.clone());
            }
        }

        let mut messages = Vec::new();
        if new_count > 0 {
            messages.push(format!("{new_count} new rules"))
        }
        if removed_count > 0 {
            messages.push(format!("{removed_count} removed rules"))
        }
        if modified_count > 0 {
            messages.push(format!("{modified_count} modified rules"))
        }
        if lingering_links > 0 {
            messages.push(format!("{lingering_links} lingering links destroyed"))
        }
        if linger_changed || !messages.is_empty() {
            let mut message = vec!["config updated".to_owned()];
            if !messages.is_empty() {
                message.push(messages.join(", "))
            }
            info!("{}", message.join(": "));
        }

        debug!("config checked");
    }

    fn new_port(&mut self, port: Port) {
        self.new_port_for_rules(port, self.rules.keys().cloned().collect())
    }

    fn port_deleted(&mut self, port: &Port) {
        use pipeswitch_lib::types::Direction;
        match &port.direction {
            Direction::Input => {
                for rule in self.rules.values_mut() {
                    rule.input.delete_port(port);
                }
            }
            Direction::Output => {
                for rule in self.rules.values_mut() {
                    rule.output.delete_port(port);
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
        for (_, rule) in self.rules.iter_mut().filter(|(n, _)| {
            // dbg!(&n, &rules, rules.contains(*n));
            rules.contains(*n)
        }) {
            let (r1, r2) = if let Direction::Input = port.direction {
                (&mut rule.input, &mut rule.output)
            } else {
                (&mut rule.output, &mut rule.input)
            };
            if r1.add_if_matches(&port, &state) {
                for old_port_id in &r2.matching_ports {
                    let old_port = state.ports.get(old_port_id).unwrap().clone();
                    if r1.should_ignore_channel(r2) || port.channel == old_port.channel {
                        let op_alias = old_port.alias.clone();
                        let (i_name, o_name) = if let Direction::Input = port.direction {
                            (&port.alias, &op_alias)
                        } else {
                            (&op_alias, &port.alias)
                        };
                        drop(state);
                        if let Some(link) = self
                            .pipeswitch
                            .create_link(port.clone(), old_port, rule.name.clone())
                            .unwrap()
                        {
                            let link_id = link.id;
                            info!("connected {o_name} to {i_name} ({link_id})");
                        }
                        state = self.pipeswitch.lock_current_state();
                    }
                }
            }
        }
    }
}
