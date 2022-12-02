use std::collections::HashSet;

use log::*;
use pipeswitch_lib::{
    config::{LinkConfig, NodeOrTarget},
    types::Port,
    PipewireState,
};
use regex::{Regex, RegexBuilder};

#[derive(Debug)]
pub struct LinkRules {
    pub name: String,
    pub input: Rule,
    pub output: Rule,
    pub links: HashSet<u32>,
}

impl From<(String, LinkConfig)> for LinkRules {
    fn from((name, cfg): (String, LinkConfig)) -> Self {
        LinkRules {
            name: name.clone(),
            input: Rule::from_node_or_target(name.clone(), cfg.special_empty_ports, &cfg.sink),
            output: Rule::from_node_or_target(name, cfg.special_empty_ports, &cfg.source),
            links: HashSet::new(),
        }
    }
}

#[derive(Debug)]
pub struct Rule {
    pub name: String,
    pub client: Option<Regex>,
    pub node: Option<Regex>,
    pub port: Option<Regex>,
    pub matching_ports: HashSet<u32>,
    pub special_empty_ports: bool,
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
                    RegexBuilder::new(node_name)
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
                    RegexBuilder::new(rex)
                        .case_insensitive(true)
                        .build()
                        .unwrap()
                }),
                node: t.node.as_ref().map(|rex| {
                    RegexBuilder::new(rex)
                        .case_insensitive(true)
                        .build()
                        .unwrap()
                }),
                port: t.port.as_ref().map(|rex| {
                    RegexBuilder::new(rex)
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
    pub fn add_if_matches(&mut self, port: &Port, state: &PipewireState) -> bool {
        let port_matches = match &self.port {
            Some(regex) => matches_entirely(regex, &port.name).unwrap_or(false),
            _ => true,
        };

        if port_matches {
            let node = state.nodes.get(&port.node_id);
            let client = node.and_then(|n| state.clients.get(&n.client_id));

            let node_matches = match (&self.node, node) {
                (Some(regex), Some(node)) => {
                    matches_entirely(regex, &node.node_name).unwrap_or(false)
                }
                (Some(_), None) => false,
                _ => true,
            };
            let client_matches = match (&self.client, client) {
                (Some(regex), Some(client)) => {
                    matches_entirely(regex, &client.application_name).unwrap_or(false)
                }
                (Some(_), None) => false,
                _ => true,
            };

            if node_matches && client_matches {
                self.matching_ports.insert(port.id);
                let alias = &port.alias;
                let direction = &port.direction;
                let name = &self.name;
                debug!("new {direction:?} for [{name}] ({alias})");
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn delete_port(&mut self, port: &Port) -> bool {
        let was = self.matching_ports.remove(&port.id);
        if was {
            let alias = &port.alias;
            let direction = &port.direction;
            let name = &self.name;
            debug!("removed {direction:?} from [{name}] ({alias})");
        }
        was
    }

    pub fn should_ignore_channel(&self, other: &Rule) -> bool {
        let ports_some = self.port.is_some() || other.port.is_some();
        !self.special_empty_ports || ports_some
    }
}
