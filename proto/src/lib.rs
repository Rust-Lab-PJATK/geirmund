use prost::{Message, Oneof};
use std::ops::{Deref, DerefMut};

pub mod master;
pub mod worker;

// This is just a wrapper type for `ProtoResult`
// as enums can not be passed as `Message` with `prost` crate.
// This type should not be constructed directly
// but rather with `ProtoResult::into()`.
// TODO: find a workaround to skip wrapping enums with struct `Message`
#[derive(Message, Copy, Clone, PartialEq, Eq)]
pub struct ProtoResultWrapper<T, E>
where
    T: Message + Default + Clone + PartialEq + Eq,
    E: Message + Default + Clone + PartialEq + Eq,
{
    // Must be wrapped with an `Option` as `prost` crate requires so.
    // TODO: find a workaround to skip wrapping enums with an `Option`
    #[prost(oneof = "ProtoResult", tags = "1, 2")]
    result: Option<ProtoResult<T, E>>,
}

#[derive(Oneof, Copy, Clone, PartialEq, Eq)]
pub enum ProtoResult<T, E>
where
    T: Message + Default + Clone + PartialEq + Eq,
    E: Message + Default + Clone + PartialEq + Eq,
{
    #[prost(message, tag = "1")]
    Ok(T),
    #[prost(message, tag = "2")]
    Err(E),
}

impl<T, E> Into<ProtoResultWrapper<T, E>> for ProtoResult<T, E>
where
    T: Message + Default + Clone + PartialEq + Eq,
    E: Message + Default + Clone + PartialEq + Eq,
{
    fn into(self) -> ProtoResultWrapper<T, E> {
        ProtoResultWrapper { result: Some(self) }
    }
}

impl<T, E> Into<Option<ProtoResult<T, E>>> for ProtoResultWrapper<T, E>
where
    T: Message + Default + Clone + PartialEq + Eq,
    E: Message + Default + Clone + PartialEq + Eq,
{
    fn into(self) -> Option<ProtoResult<T, E>> {
        self.result
    }
}

impl<T, E> Deref for ProtoResultWrapper<T, E>
where
    T: Message + Default + Clone + PartialEq + Eq,
    E: Message + Default + Clone + PartialEq + Eq,
{
    type Target = Option<ProtoResult<T, E>>;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

impl<T, E> DerefMut for ProtoResultWrapper<T, E>
where
    T: Message + Default + Clone + PartialEq + Eq,
    E: Message + Default + Clone + PartialEq + Eq,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.result
    }
}
