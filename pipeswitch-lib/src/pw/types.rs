use std::collections::HashMap;

use pipewire::{
    keys::*,
    link::LinkInfo,
    registry::GlobalObject,
    spa::{ForeignDict, ReadableDict},
    types::ObjectType,
};

use super::PipewireError;

pub const VERSION: u32 = 3;
pub const KEY_RULE_NAME: &str = "pipeswitch.rule.name";

type PwIdType = u32;

#[derive(Debug, Clone)]
pub enum Direction {
    Input,
    Output,
}

impl Direction {
    fn from<T: Into<String>>(input: T) -> Result<Self, PipewireError> {
        let input = input.into();
        match input.as_str() {
            "in" => Ok(Direction::Input),
            "out" => Ok(Direction::Output),
            _ => Err(PipewireError::InvalidDirection(input)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    Left,
    Right,
    Mono,
}

impl Channel {
    fn from_channel<T: Into<String>>(input: Option<T>) -> Result<Option<Self>, PipewireError> {
        if let Some(input) = input {
            let input = input.into();
            Ok(Some(match input.as_str() {
                "FL" => Channel::Left,
                "FR" => Channel::Right,
                "MONO" => Channel::Mono,
                _ => Err(PipewireError::InvalidChannel(input))?,
            }))
        } else {
            Ok(None)
        }
    }

    fn from_portid(input: u32) -> Result<Self, PipewireError> {
        Ok(match input {
            0 => Channel::Left,
            1 => Channel::Right,
            _ => Err(PipewireError::InvalidChannel(format!("port.id {input}")))?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Port {
    pub id: PwIdType,
    /// Usually 0 or 1
    pub local_port_id: PwIdType,
    pub path: Option<String>,
    pub node_id: PwIdType,
    pub dsp: Option<String>,
    pub channel: Channel,
    pub name: String,
    pub direction: Direction,
    pub alias: String,
    pub physical: Option<bool>,
    pub terminal: Option<bool>,
}

impl Port {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Self, PipewireError> {
        let props = global.props.as_ref().ok_or(PipewireError::MissingProps(
            global.id,
            ObjectType::Port,
            map_props(global.props.as_ref().unwrap()),
        ))?;
        let get_prop = |property| props.get(property).map(|v| v.to_string());
        let get_prop_or = |property| {
            get_prop(property).ok_or(PipewireError::PropNotFound(
                global.id,
                ObjectType::Port,
                map_props(props),
                property,
            ))
        };
        let local_port_id = get_prop_or(*PORT_ID)?.parse()?;
        Ok(Port {
            id: global.id,
            local_port_id,
            path: get_prop(*OBJECT_PATH),
            node_id: get_prop_or(*NODE_ID)?.parse()?,
            dsp: get_prop(*FORMAT_DSP),
            channel: Channel::from_channel(get_prop(*AUDIO_CHANNEL))?
                .unwrap_or(Channel::from_portid(local_port_id)?),
            name: get_prop_or(*PORT_NAME)?,
            direction: Direction::from(get_prop_or(*PORT_DIRECTION)?)?,
            alias: get_prop_or(*PORT_ALIAS)?,
            physical: get_prop(*PORT_PHYSICAL).map(|v| v.parse()).transpose()?,
            terminal: get_prop(*PORT_TERMINAL).map(|v| v.parse()).transpose()?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: PwIdType,
    pub path: Option<String>,
    pub factory_id: Option<PwIdType>,
    pub client_id: PwIdType,
    pub device_id: Option<PwIdType>,
    pub application_name: Option<String>,
    pub node_description: Option<String>,
    pub node_name: String,
    pub node_nick: Option<String>,
    pub media_type: Option<String>,
    pub media_category: Option<String>,
    pub media_class: Option<String>,
    pub media_role: Option<String>,
}

impl Node {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Self, PipewireError> {
        let props = global.props.as_ref().ok_or(PipewireError::MissingProps(
            global.id,
            ObjectType::Node,
            map_props(global.props.as_ref().unwrap()),
        ))?;
        let get_prop = |property| props.get(property).map(|v| v.to_string());
        let get_prop_or = |property| {
            get_prop(property).ok_or(PipewireError::PropNotFound(
                global.id,
                ObjectType::Node,
                map_props(props),
                property,
            ))
        };

        Ok(Node {
            id: global.id,
            path: get_prop(*OBJECT_PATH),
            factory_id: get_prop(*FACTORY_ID).map(|v| v.parse()).transpose()?,
            client_id: get_prop_or(*CLIENT_ID)?.parse()?,
            device_id: get_prop(*DEVICE_ID).map(|v| v.parse()).transpose()?,
            application_name: get_prop(*APP_NAME),
            node_description: get_prop(*NODE_DESCRIPTION),
            node_name: get_prop_or(*NODE_NAME)?,
            node_nick: get_prop(*NODE_NICK),
            media_type: get_prop(*MEDIA_TYPE),
            media_category: get_prop(*MEDIA_CATEGORY),
            media_class: get_prop(*MEDIA_CLASS),
            media_role: get_prop(*MEDIA_ROLE),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Link {
    pub id: PwIdType,
    pub factory_id: PwIdType,
    pub client_id: Option<PwIdType>,
    pub output_node: PwIdType,
    pub output_port: PwIdType,
    pub input_node: PwIdType,
    pub input_port: PwIdType,
    pub rule_name: Option<String>,
}

impl Link {
    pub fn from_link_info(link_info: &LinkInfo) -> Result<Self, PipewireError> {
        let props = link_info.props();
        let props = link_info.props().ok_or(PipewireError::MissingProps(
            link_info.id(),
            ObjectType::Link,
            map_props(props.as_ref().unwrap()),
        ))?;
        let get_prop = |property| props.get(property).map(|v| v.to_string());
        let get_prop_or = |property| {
            get_prop(property).ok_or(PipewireError::PropNotFound(
                link_info.id(),
                ObjectType::Link,
                map_props(props),
                property,
            ))
        };
        Ok(Link {
            id: link_info.id(),
            factory_id: get_prop_or(*FACTORY_ID)?.parse()?,
            client_id: get_prop(*CLIENT_ID).map(|v| v.parse()).transpose()?,
            output_node: link_info.output_node_id(),
            output_port: link_info.output_port_id(),
            input_node: link_info.input_node_id(),
            input_port: link_info.input_port_id(),
            rule_name: get_prop(KEY_RULE_NAME),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    pub id: PwIdType,
    pub module_id: PwIdType,
    pub protocol: String,
    pub pid: PwIdType,
    pub uid: PwIdType,
    pub gid: PwIdType,
    pub label: String,
    pub application_name: String,
}

impl Client {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Self, PipewireError> {
        let props = global.props.as_ref().ok_or(PipewireError::MissingProps(
            global.id,
            ObjectType::Client,
            map_props(global.props.as_ref().unwrap()),
        ))?;
        let get_prop = |property| props.get(property).map(|v| v.to_string());
        let get_prop_or = |property| {
            get_prop(property).ok_or(PipewireError::PropNotFound(
                global.id,
                ObjectType::Client,
                map_props(props),
                property,
            ))
        };

        Ok(Client {
            id: global.id,
            module_id: get_prop_or(*MODULE_ID)?.parse()?,
            protocol: get_prop_or(*PROTOCOL)?,
            pid: get_prop_or(*SEC_PID)?.parse()?,
            uid: get_prop_or(*SEC_UID)?.parse()?,
            gid: get_prop_or(*SEC_GID)?.parse()?,
            label: get_prop_or(*SEC_LABEL)?,
            application_name: get_prop_or(*APP_NAME)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Factory {
    pub id: PwIdType,
    pub module_id: PwIdType,
    pub name: String,
    pub type_name: String,
}

impl Factory {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Self, PipewireError> {
        let props = global.props.as_ref().ok_or(PipewireError::MissingProps(
            global.id,
            ObjectType::Factory,
            map_props(global.props.as_ref().unwrap()),
        ))?;
        let get_prop = |property| props.get(property).map(|v| v.to_string());
        let get_prop_or = |property| {
            get_prop(property).ok_or(PipewireError::PropNotFound(
                global.id,
                ObjectType::Factory,
                map_props(props),
                property,
            ))
        };

        Ok(Factory {
            id: global.id,
            module_id: get_prop_or(*MODULE_ID)?.parse()?,
            name: get_prop_or(*FACTORY_NAME)?,
            type_name: get_prop_or(*FACTORY_TYPE_NAME)?,
        })
    }
}

#[derive(Clone, Debug)]
pub enum Object {
    Port(Port),
    Node(Node),
    Link(Link),
    Client(Client),
    Factory(Factory),
}

impl Object {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Option<Self>, PipewireError> {
        if global.version != VERSION {
            Err(PipewireError::InvalidVersion(global.version))?
        }
        match global.type_ {
            ObjectType::Port => Ok(Some(Self::Port(Port::from_global(global)?))),
            ObjectType::Node => Ok(Some(Self::Node(Node::from_global(global)?))),
            ObjectType::Client => Ok(Some(Self::Client(Client::from_global(global)?))),
            ObjectType::Factory => Ok(Some(Self::Factory(Factory::from_global(global)?))),
            _ => Ok(None),
        }
    }
}

fn map_props(props: &ForeignDict) -> HashMap<String, String> {
    props
        .iter()
        .map(|(v1, v2)| (v1.into(), v2.into()))
        .collect()
}
