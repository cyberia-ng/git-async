use alloc::vec::Vec;

use crate::object::ObjectId;

mod index;
mod lookup;
mod loose;
mod pack;

#[derive(Debug, PartialEq, Eq)]
enum RawObjectType {
    Commit,
    Tag,
    Blob,
    Tree,
}

#[derive(Debug)]
struct RawObject {
    pub object_type: RawObjectType,
    pub id: ObjectId,
    pub body: Vec<u8>,
}
