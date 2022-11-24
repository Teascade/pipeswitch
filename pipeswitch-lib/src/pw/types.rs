use pipewire::{
    keys::*,
    registry::GlobalObject,
    spa::{ForeignDict, ReadableDict},
    types::ObjectType,
};

use crate::PipewireError;

pub const VERSION: u32 = 3;

type PwIdType = u32;

#[derive(Debug)]
pub struct Port {
    pub id: PwIdType,
    pub port_id: PwIdType,
    pub path: Option<String>,
    pub node_id: PwIdType,
    pub dsp: Option<String>,
    pub channel: Option<String>,
    pub name: String,
    pub direction: String,
    pub alias: String,
    pub physical: Option<bool>,
    pub terminal: Option<bool>,
}

impl Port {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Self, PipewireError> {
        let props = global
            .props
            .as_ref()
            .ok_or(PipewireError::MissingProps(global.id))?;
        let get_prop = |property| {
            props
                .get(property)
                .ok_or(PipewireError::PropNotFound(property))
        };
        Ok(Port {
            id: global.id,
            port_id: get_prop(*PORT_ID)?.parse()?,
            path: props.get(*OBJECT_PATH).map(|v| v.to_string()),
            node_id: get_prop(*NODE_ID)?.parse()?,
            dsp: props.get(*FORMAT_DSP).map(|v| v.to_string()),
            channel: props.get(*AUDIO_CHANNEL).map(|v| v.to_string()),
            name: get_prop(*PORT_NAME)?.to_string(),
            direction: get_prop(*PORT_DIRECTION)?.to_string(),
            alias: get_prop(*PORT_ALIAS)?.to_string(),
            physical: props.get(*PORT_PHYSICAL).map(|v| v.parse()).transpose()?,
            terminal: props.get(*PORT_TERMINAL).map(|v| v.parse()).transpose()?,
        })
    }
}

pub(crate) enum PipewireObject {
    Port(Port),
}

impl PipewireObject {
    pub fn from_global(global: &GlobalObject<ForeignDict>) -> Result<Option<Self>, PipewireError> {
        if global.version != VERSION {
            Err(PipewireError::InvalidVersion(global.version))?
        }
        match global.type_ {
            ObjectType::Port => Ok(Some(Self::Port(Port::from_global(global)?))),
            _ => Ok(None),
        }
    }
}
