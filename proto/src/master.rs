use prost::{Enumeration, Message, Oneof};

#[derive(Message, Clone, PartialEq, Eq)]
pub struct Packet {
    // Must be wrapped with an `Option` as `prost` crate requires so.
    #[prost(oneof = "MasterMessage", tags = "1, 2")]
    pub msg: Option<MasterMessage>,
}

#[derive(Oneof, Clone, PartialEq, Eq)]
pub enum MasterMessage {
    #[prost(message, tag = "1")]
    LoadCommand(LoadCommand),

    #[prost(message, tag = "2")]
    GenerateCommand(GenerateCommand),
}

#[derive(Message, Copy, Clone, Eq, PartialEq)]
pub struct LoadCommand {
    #[prost(uint32, tag = "1")]
    pub id: u32,

    #[prost(enumeration = "ModelType", tag = "2")]
    pub r#type: i32,
}

#[derive(Enumeration, Debug, Copy, Clone, PartialEq, Eq)]
#[repr(i32)]
pub enum ModelType {
    Llama3v2_1B = 0,
}

#[derive(Message, Clone, PartialEq, Eq)]
pub struct GenerateCommand {
    #[prost(uint32, tag = "1")]
    pub id: u32,

    #[prost(enumeration = "ModelType", tag = "2")]
    pub r#type: i32,

    #[prost(string, tag = "3")]
    pub prompt: String,
}

impl Packet {
    pub fn new(msg: MasterMessage) -> Self {
        Self { msg: Some(msg) }
    }

    pub fn new_load_command(cmd: LoadCommand) -> Self {
        Self {
            msg: Some(MasterMessage::LoadCommand(cmd)),
        }
    }

    pub fn new_generate_command(cmd: GenerateCommand) -> Self {
        Self {
            msg: Some(MasterMessage::GenerateCommand(cmd)),
        }
    }
}

impl LoadCommand {
    pub fn new(id: u32, model_type: ModelType) -> Self {
        Self {
            id,
            r#type: model_type as i32,
        }
    }
}

impl GenerateCommand {
    pub fn new(id: u32, model_type: ModelType, prompt: String) -> Self {
        Self {
            id,
            r#type: model_type as i32,
            prompt,
        }
    }
}
