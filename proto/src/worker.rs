use crate::{ProtoResult, ProtoResultWrapper};
use prost::{Message, Oneof};


#[derive(Message, Clone, Eq, PartialEq)]
pub struct Packet {
    // Must be wrapped with an `Option` as `prost` crate requires so.
    #[prost(oneof = "WorkerMessage", tags = "1, 2")]
    pub msg: Option<WorkerMessage>,
}

#[derive(Oneof, Clone, PartialEq, Eq)]
pub enum WorkerMessage {
    #[prost(message, tag = "1")]
    GenerateResponse(ProtoResultWrapper<GenerateResponse, WorkerErrorWrapper>),

    #[prost(message, tag = "2")]
    LoadResponse(ProtoResultWrapper<LoadResponse, WorkerErrorWrapper>),
}

#[derive(Message, Clone, PartialEq, Eq)]
pub struct GenerateResponse {
    #[prost(uint32, tag = "1")]
    pub id: u32,

    #[prost(string, tag = "2")]
    pub content: String,
}

#[derive(Message, Copy, Clone, PartialEq, Eq)]
pub struct LoadResponse {
    #[prost(uint32, tag = "1")]
    pub id: u32,
}

// This is just a wrapper type for `WorkerError`
// as enums can not be passed as `Message` with `prost` crate.
// This type should not be constructed directly
// but rather with `WorkerError::into()`.
#[derive(Message, Clone, PartialEq, Eq)]
pub struct WorkerErrorWrapper {
    #[prost(oneof = "WorkerError", tags = "1, 2, 3, 4, 5")]
    pub err: Option<WorkerError>,
}

#[derive(Oneof, Clone, PartialEq, Eq)]
pub enum WorkerError {
    #[prost(message, tag = "1")]
    ModelBusy(WorkerErrorContent),

    #[prost(message, tag = "2")]
    ModelAlreadyLoaded(WorkerErrorContent),

    #[prost(message, tag = "3")]
    ModelNotLoaded(WorkerErrorContent),

    #[prost(message, tag = "4")]
    LoadingError(WorkerErrorContent),

    #[prost(message, tag = "5")]
    GenerationError(WorkerErrorContent),
}

#[derive(Message, Clone, PartialEq, Eq)]
pub struct WorkerErrorContent {
    #[prost(uint32, tag = "1")]
    id: u32,

    #[prost(string, optional, tag = "2")]
    content: Option<String>,
}

impl Packet {
    pub fn new(msg: WorkerMessage) -> Self {
        Self { msg: Some(msg) }
    }

    pub fn new_load_response(response: ProtoResult<LoadResponse, WorkerErrorWrapper>) -> Self {
        Self {
            msg: Some(WorkerMessage::LoadResponse(response.into())),
        }
    }

    pub fn new_generate_response(
        response: ProtoResult<GenerateResponse, WorkerErrorWrapper>,
    ) -> Self {
        Self {
            msg: Some(WorkerMessage::GenerateResponse(response.into())),
        }
    }
}

impl GenerateResponse {
    pub fn new(id: u32, content: String) -> Self {
        Self { id, content }
    }
}

impl LoadResponse {
    pub fn new(id: u32) -> Self {
        Self { id }
    }
}

impl WorkerErrorContent {
    pub fn new(id: u32) -> Self {
        Self { id, content: None }
    }

    pub fn new_with_content(id: u32, content: String) -> Self {
        Self {
            id,
            content: Some(content),
        }
    }
}

impl Into<WorkerErrorWrapper> for WorkerError {
    fn into(self) -> WorkerErrorWrapper {
        WorkerErrorWrapper { err: Some(self) }
}
