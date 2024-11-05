use prost::{Message, Oneof};

#[derive(Message)]
pub struct Package {
    #[prost(oneof = "Command", tags = "1")]
    pub cmd: Option<Command>,
}

#[derive(Oneof)]
pub enum Command {
    #[prost(message, tag = "1")]
    LoadCommand(Load),
}

#[derive(Message)]
pub struct Load {
    #[prost(enumeration = "ModelType", tag = "1")]
    pub r#type: i32,
}

#[derive(prost::Enumeration, Debug)]
#[repr(i32)]
pub enum ModelType {
    Llama3v2_1B = 0,
}

impl Package {
    pub fn new(cmd: Command) -> Self {
        Self { cmd: Some(cmd) }
    }
}

impl Load {
    pub fn new(model_type: ModelType) -> Self {
        Self {
            r#type: model_type as i32,
        }
    }
}
