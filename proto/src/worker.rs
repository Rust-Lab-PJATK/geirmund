use prost::{Message, Oneof};

#[derive(Message)]
pub struct Packet {
    #[prost(oneof = "WorkerMessage", tags = "1")]
    pub msg: Option<WorkerMessage>,
}

#[derive(Oneof)]
pub enum WorkerMessage {
    #[prost(message, tag = "1")]
    GenerateResponse(Response),
}

#[derive(Message)]
pub struct Response {
    #[prost(int32, tag = "1")]
    pub id: i32,

    #[prost(string, tag = "2")]
    pub content: String,
}

impl Packet {
    pub fn new(msg: WorkerMessage) -> Self {
        Self { msg: Some(msg) }
    }
}

impl Response {
    pub fn new(id: i32, content: String) -> Self {
        Self { id, content }
    }
}
