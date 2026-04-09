use alloc::vec::Vec;

use crate::object::ObjectId;

mod index;
pub(crate) mod lookup;
mod loose;
mod pack;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ObjectType {
    Commit,
    Tag,
    Blob,
    Tree,
}

#[derive(Debug)]
pub(crate) struct RawObject {
    pub object_type: ObjectType,
    pub id: ObjectId,
    pub body: Vec<u8>,
}
