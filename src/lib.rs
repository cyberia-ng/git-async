#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod directory;
pub mod error;
mod object;
mod object_store;
mod parsing;
mod reference;
mod repo;
#[cfg(feature = "serde")]
mod serde;

pub use directory::{DirEntry, Directory, DirectoryError, File, Offset};
pub use object::{
    CommitFields, Object, ObjectBody, ObjectHeader, ObjectId, PeeledCommit, PeeledTree, TagFields,
    TagType, TreeEntry, TreeEntryType, TreeFields,
};
pub use object_store::{ObjectSize, ObjectType};
pub use reference::{Ref, RefName, RefType};
pub use repo::Repo;

#[cfg(test)]
mod test {
    pub(crate) mod helpers;
    pub(crate) mod repo;
}
