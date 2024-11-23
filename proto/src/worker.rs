use crate::{ProtoResult, ProtoResultWrapper};
use prost::{Enumeration, Message, Oneof};
use std::fmt::Debug;

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
#[derive(Message, Copy, Clone, PartialEq, Eq)]
pub struct WorkerErrorWrapper {
    #[prost(enumeration = "WorkerError", tag = "1")]
    pub r#err: i32,
}

#[derive(Enumeration, Debug, Copy, Clone, PartialEq, Eq)]
#[repr(i32)]
pub enum WorkerError {
    ModelBusy = 0,
    ModelAlreadyLoaded = 1,
    ModelNotLoaded = 2,
    LoadingError = 3,
    GenerationError = 4,
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

impl Into<WorkerErrorWrapper> for WorkerError {
    fn into(self) -> WorkerErrorWrapper {
        WorkerErrorWrapper { r#err: self as i32 }
    }
}
