#![cfg_attr(not(test), no_std)]
extern crate alloc;

mod directory;
mod error;
mod object;
mod object_store;
mod parsing;
mod reference;
mod repo;
#[cfg(feature = "serde")]
mod serde;

pub use directory::{DirEntry, Directory, DirectoryError, File, Offset};
pub use error::{Error, GResult};
pub use object::{
    Blob, Commit, Object, ObjectHeader, ObjectId, Tag, TagType, Tree, TreeEntry, TreeEntryType,
};
pub use object_store::{ObjectSize, ObjectType};
pub use reference::{Ref, RefName, RefType};
pub use repo::Repo;

#[cfg(test)]
mod test {
    pub(crate) mod helpers;
    pub(crate) mod repo;
}
