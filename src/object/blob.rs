use crate::object::{Object, ObjectId};
use accessory::Accessors;
use alloc::vec::Vec;

/// A blob object
///
/// Represents arbitrary data, e.g. the contents of a file
#[derive(Debug, Clone, Accessors)]
pub struct Blob {
    /// The [`ObjectId`] of the blob object
    #[access(get(cp))]
    id: ObjectId,

    /// The data that the blob contains
    #[access(get(ty(&[u8])))]
    data: Vec<u8>,
}

impl PartialEq for Blob {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Blob {}
impl PartialOrd for Blob {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Blob {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl Blob {
    pub(crate) fn new(id: ObjectId, data: Vec<u8>) -> Self {
        Blob { id, data }
    }

    /// Move the data out of the blob object.
    pub fn data_owned(self) -> Vec<u8> {
        self.data
    }

    /// Wrap the [`Blob`] as a generic [`Object`].
    pub fn as_object(self) -> Object {
        Object::Blob(self)
    }
}
