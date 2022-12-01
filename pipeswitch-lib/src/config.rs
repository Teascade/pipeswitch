use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, str::FromStr};
use toml_edit::{table, Document, Item, Value};

use crate::PipeswitchError;

pub const DEFAULT_CONFIG_NAME: &str = "pipeswitch.conf";

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub general: General,
    #[serde(rename = "link")]
    pub links: HashMap<String, Link>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct General {
    /// keep links that dont exist in the config anymore
    pub linger_links: bool,
    /// inotify listen config and reload when it changes
    pub hotreload_config: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Link {
    #[serde(rename = "in")]
    pub input: NodeOrTarget,
    #[serde(rename = "out")]
    pub output: NodeOrTarget,
    /// if false, empty port fields on both sides are never treated specially channel-wise
    special_empty_ports: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum NodeOrTarget {
    NodeName(String),
    Target(Target),
}

#[derive(Serialize, Default, Deserialize, Debug)]
pub struct Target {
    pub client: Option<String>,
    pub node: Option<String>,
    pub port: Option<String>,
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        config_dir().map(|dir| dir.join(DEFAULT_CONFIG_NAME))
    }

    pub fn load_from(path: &PathBuf) -> Result<(Config, Document), PipeswitchError> {
        Ok(Config::from_string(&fs::read_to_string(path)?)?)
    }

    pub fn write_to(&self, path: &PathBuf, doc: Option<Document>) -> Result<(), PipeswitchError> {
        let text = Config::to_string(&self, doc)?;
        Ok(fs::write(path, text)?)
    }

    pub fn to_string(&self, old_document: Option<Document>) -> Result<String, PipeswitchError> {
        let mut document = toml_edit::ser::to_document(&self)?;
        // General
        let mut general_item = Item::Table(
            document
                .remove("general")
                .and_then(|v| v.into_table().ok())
                .ok_or(PipeswitchError::Unknown)?,
        );

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

        if let Some(old_document) = old_document {
            clone_decor(&mut general_item, old_document.get("general"));
            clone_decor(&mut link_item, old_document.get("link"));
            document.set_trailing(old_document.trailing());
        }

        document.insert("general", general_item);
        document.insert("link", link_item);
        Ok(document.to_string())
    }

    pub fn from_string(input: &str) -> Result<(Self, Document), PipeswitchError> {
        let document = Document::from_str(input)?;
        Ok((toml_edit::de::from_document(document.clone())?, document))
    }
}

pub fn clone_decor(to: &mut Item, from: Option<&Item>) {
    use Item::*;
    if let Some(from) = from {
        match (to, from) {
            (Value(to_val), Value(from_val)) => clone_value_decor(to_val, from_val),
            (Table(to_table), Table(from_table)) => {
                *to_table.decor_mut() = from_table.decor().clone();
                for (mut key, to_item) in to_table.iter_mut() {
                    if let Some((from_key, from_item)) = from_table.get_key_value(&key) {
                        *key.decor_mut() = from_key.decor().clone();
                        clone_decor(to_item, Some(from_item));
                    }
                }
            }
            (ArrayOfTables(to_tables), ArrayOfTables(from_tables)) => {
                for (to, from) in to_tables.iter_mut().zip(from_tables) {
                    *to.decor_mut() = from.decor().clone();
                    for (mut key, to_item) in to.iter_mut() {
                        if let Some((from_key, from_item)) = from.get_key_value(&key) {
                            *key.decor_mut() = from_key.decor().clone();
                            clone_decor(to_item, Some(from_item));
                        }
                    }
                }
            }
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
