use prost::{Enumeration, Message, Oneof};

#[derive(Message)]
pub struct Packet {
    #[prost(oneof = "MasterMessage", tags = "1, 2")]
    pub msg: Option<MasterMessage>,
}

#[derive(Oneof)]
pub enum MasterMessage {
    #[prost(message, tag = "1")]
    LoadCommand(Load),

    #[prost(message, tag = "2")]
    GenerateCommand(Generate),
}

#[derive(Message)]
pub struct Load {
    #[prost(enumeration = "ModelType", tag = "1")]
    pub r#type: i32,
}

#[derive(Enumeration, Debug)]
#[repr(i32)]
pub enum ModelType {
    Llama3v2_1B = 0,
}

#[derive(Message)]
pub struct Generate {
    #[prost(int32, tag = "1")]
    pub id: i32,

    #[prost(enumeration = "ModelType", tag = "2")]
    pub r#type: i32,

    #[prost(string, tag = "3")]
    pub prompt: String,
}

impl Packet {
    pub fn new(msg: MasterMessage) -> Self {
        Self { msg: Some(msg) }
    }
}

impl Load {
    pub fn new(model_type: ModelType) -> Self {
        Self {
            r#type: model_type as i32,
        }
    }
}

impl Generate {
    pub fn new(id: i32, model_type: ModelType, prompt: String) -> Self {
        Self {
            id,
            r#type: model_type as i32,
            prompt,
        }
    }
}
