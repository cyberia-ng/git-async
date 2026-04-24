use crate::object::ObjectId;
use accessory::Accessors;
use alloc::vec::Vec;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Blob {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
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

    pub fn data_owned(self) -> Vec<u8> {
        self.data
    }
}
