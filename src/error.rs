use crate::{directory::DirectoryError, object::ObjectId, reference::RefName};
use alloc::vec::Vec;
use miniz_oxide::inflate::DecompressError;

pub type GResult<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Directory(DirectoryError),
    PathError(Vec<u8>),
    DecompressError(DecompressError),

    MalformedObject(ObjectId),
    MalformedRef(RefName),
    FromHexError(hex::FromHexError),
}

impl From<DirectoryError> for Error {
    fn from(value: DirectoryError) -> Self {
        Self::Directory(value)
    }
}

impl From<DecompressError> for Error {
    fn from(value: DecompressError) -> Self {
        Self::DecompressError(value)
    }
}

impl From<hex::FromHexError> for Error {
    fn from(value: hex::FromHexError) -> Self {
        Self::FromHexError(value)
    }
}
