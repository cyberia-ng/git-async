use crate::{directory::DirectoryError, object::ObjectId, reference::RefName};
use alloc::vec::Vec;
use miniz_oxide::inflate::DecompressError;

#[cfg(feature = "serde")]
use serde::Serialize;

pub type GResult<T> = core::result::Result<T, Error>;

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum Error {
    Directory(#[cfg_attr(feature = "serde", serde(skip))] DirectoryError),
    PathError(#[cfg_attr(feature = "serde", serde(with = "serde_bytes"))] Vec<u8>),
    DecompressError(#[cfg_attr(feature = "serde", serde(skip))] DecompressError),

    MalformedObject(ObjectId),
    MalformedRef(RefName),
    FromHexError(#[cfg_attr(feature = "serde", serde(skip))] hex::FromHexError),

    UnsupportedIndexVersion,
    UnsupportedPackVersion,
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
