use crate::object::ObjectId;
use alloc::vec::Vec;
#[cfg(feature = "serde")]
use serde::Serialize;

pub(crate) mod cache;
mod index;
pub(crate) mod lookup;
mod loose;
mod pack;
pub(crate) mod page_read;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum ObjectType {
    Commit,
    Tag,
    Blob,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectSize(pub u64);

#[derive(Debug)]
pub(crate) struct RawObject {
    pub object_type: ObjectType,
    pub id: ObjectId,
    pub body: Vec<u8>,
}
