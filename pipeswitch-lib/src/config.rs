use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, str::FromStr};
use toml_edit::{table, Document, Item, Value};

use crate::PipeswitchError;

const DEFAULT_CONFIG_NAME: &str = "pipeswitch.conf";
const DEFAULT_CONFIG: &str = include_str!("default.toml");

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub general: General,
    pub log: Logging,
    #[serde(rename = "link")]
    pub links: HashMap<String, LinkConfig>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct General {
    /// keep links that dont exist in the config anymore
    pub linger_links: bool,
    /// inotify listen config and reload when it changes
    pub hotreload_config: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Logging {
    pub level: log::Level,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LinkConfig {
    #[serde(rename = "in")]
    pub input: NodeOrTarget,
    #[serde(rename = "out")]
    pub output: NodeOrTarget,
    /// if false, empty port fields on both sides are never treated specially channel-wise
    #[serde(default = "return_true")]
    pub special_empty_ports: bool,
}

const fn return_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum NodeOrTarget {
    NodeName(String),
    Target(Target),
}

#[derive(Serialize, Default, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub client: Option<String>,
    pub node: Option<String>,
    pub port: Option<String>,
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join(DEFAULT_CONFIG_NAME))
    }

    pub fn default_conf() -> Result<(Config, Document), PipeswitchError> {
        Config::from_string(DEFAULT_CONFIG)
    }

    pub fn load_from(path: &PathBuf) -> Result<Option<(Config, Document)>, PipeswitchError> {
        if !path.try_exists()? {
            Ok(None)
        } else {
            Ok(Some(Config::from_string(&fs::read_to_string(path)?)?))
        }
    }

    pub fn write_to(&self, path: &PathBuf, doc: Option<&Document>) -> Result<(), PipeswitchError> {
        let text = Config::to_string(&self, doc)?;
        Ok(fs::write(path, text)?)
    }

    pub fn to_string(&self, old_document: Option<&Document>) -> Result<String, PipeswitchError> {
        let mut document = toml_edit::ser::to_document(&self)?;
        // General
        let general_item = Item::Table(
            document
                .remove("general")
                .and_then(|v| v.into_table().ok())
                .ok_or(PipeswitchError::Unknown)?,
        );
        // Log
        let log_item = Item::Table(
            document
                .remove("log")
                .and_then(|v| v.into_table().ok())
                .ok_or(PipeswitchError::Unknown)?,
        );
        // Link
        let mut link_item = table();
        let tableref = link_item.as_table_mut().ok_or(PipeswitchError::Unknown)?;
        tableref.set_implicit(true);
        for (internal_string, val) in document
            .remove("link")
            .and_then(|v| v.into_table().ok())
            .ok_or(PipeswitchError::Unknown)?
        {
            let table_item = Item::Table(val.into_table().map_err(|_| PipeswitchError::Unknown)?);
            tableref.insert(&internal_string, table_item);
        }
        // Insert them all
        document.insert("general", general_item);
        document.insert("log", log_item);
        document.insert("link", link_item);
        // Clone decor and return
        if let Some(old_document) = old_document {
            clone_decor(&mut document, &old_document);
        }
        Ok(document.to_string())
    }

    pub fn from_string(input: &str) -> Result<(Self, Document), PipeswitchError> {
        let document = Document::from_str(input)?;
        Ok((toml_edit::de::from_document(document.clone())?, document))
    }
}

pub fn clone_decor(to: &mut Document, from: &Document) {
    for (key, item) in to.iter_mut() {
        clone_item_decor(item, from.get(&key))
    }
    to.set_trailing(from.trailing());
}

pub fn clone_item_decor(to: &mut Item, from: Option<&Item>) {
    use Item::*;
    if let Some(from) = from {
        match (to, from) {
            (Value(to_val), Value(from_val)) => clone_value_decor(to_val, from_val),
            (Table(to_table), Table(from_table)) => {
                *to_table.decor_mut() = from_table.decor().clone();
                for (mut key, to_item) in to_table.iter_mut() {
                    if let Some((from_key, from_item)) = from_table.get_key_value(&key) {
                        *key.decor_mut() = from_key.decor().clone();
                        clone_item_decor(to_item, Some(from_item));
                    }
                }
            }
            (ArrayOfTables(to_tables), ArrayOfTables(from_tables)) => {
                for (to, from) in to_tables.iter_mut().zip(from_tables) {
                    *to.decor_mut() = from.decor().clone();
                    for (mut key, to_item) in to.iter_mut() {
                        if let Some((from_key, from_item)) = from.get_key_value(&key) {
                            *key.decor_mut() = from_key.decor().clone();
                            clone_item_decor(to_item, Some(from_item));
                        }
                    }
                }
            }
            // (Value(toml_edit::Value::InlineTable(to_table)), Table(from_table)) => {
            //     for (to, from) in to_tables.iter_mut().zip(from_tables) {
            //         *to.decor_mut() = from.decor().clone();
            //         for (mut key, to_item) in to.iter_mut() {
            //             if let Some((from_key, from_item)) = from.get_key_value(&key) {
            //                 *key.decor_mut() = from_key.decor().clone();
            //                 clone_decor(to_item, Some(from_item));
            //             }
            //         }
            //     }
            // }
            (_, _) => {}
        }
    }
}

pub fn clone_value_decor(to: &mut Value, from: &Value) {
    use Value::*;
    match (to, from) {
        (InlineTable(to_table), InlineTable(from_table)) => {
            *to_table.decor_mut() = from_table.decor().clone();
            for (key, to_inner) in to_table.iter_mut() {
                if let Some(from_inner) = from_table.get(&key) {
                    clone_value_decor(to_inner, from_inner)
                }
            }
        }
        (Array(to_arr), Array(from_arr)) => {
            *to_arr.decor_mut() = from_arr.decor().clone();
            for (to_item, from_item) in to_arr.iter_mut().zip(from_arr.iter()) {
                clone_value_decor(to_item, from_item)
            }
        }
        (Boolean(to), Boolean(from)) => {
            *to.decor_mut() = from.decor().clone();
        }
        (Datetime(to), Datetime(from)) => {
            *to.decor_mut() = from.decor().clone();
        }
        (Float(to), Float(from)) => {
            *to.decor_mut() = from.decor().clone();
        }
        (Integer(to), Integer(from)) => {
            *to.decor_mut() = from.decor().clone();
        }
        (String(to), String(from)) => {
            *to.decor_mut() = from.decor().clone();
        }
        _ => {}
    }
}
