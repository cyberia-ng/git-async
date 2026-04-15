use crate::{
    file_system::FilesystemError, object::ObjectId, parsing::ParseError, reference::RefName,
};
use alloc::vec::Vec;
use miniz_oxide::inflate::DecompressError;

#[cfg(feature = "serde")]
use serde::Serialize;

pub type GResult<T> = core::result::Result<T, Error>;

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum Error {
    FileSystem(#[cfg_attr(feature = "serde", serde(skip))] FilesystemError),
    PathError(#[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))] Vec<u8>),
    DecompressError(#[cfg_attr(feature = "serde", serde(skip))] DecompressError),
    FromHexError(#[cfg_attr(feature = "serde", serde(skip))] hex::FromHexError),
    UnsupportedIndexVersion,
    UnsupportedPackVersion,
    MalformedPackedRefs,
    MalformedRef(RefName),
    RefNotFound(RefName),
    MalformedPackObject(ObjectId),
    MalformedObject(ObjectId),
    ObjectParseError {
        id: ObjectId,
        #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
        snippet: Vec<u8>,
    },
    ObjectMissingRequiredFields(ObjectId),
    MissingObject(ObjectId),
    ObjectTooLarge(ObjectId),
    UnexpectedThinPack,
    NotAnnotatedWithRepo,
}

#[derive(Debug)]
pub(crate) enum InternalObjectError {
    ExternalError(Error),
    ObjectTooLarge,
    ParseError { snippet: Vec<u8> },
    MissingFields,
    MalformedPackObject,
}

pub(crate) type IResult<T> = core::result::Result<T, InternalObjectError>;

impl From<Error> for InternalObjectError {
    fn from(value: Error) -> Self {
        Self::ExternalError(value)
    }
}

impl From<ParseError> for InternalObjectError {
    fn from(value: ParseError) -> Self {
        match value {
            ParseError::ParseError { input_snippet } => InternalObjectError::ParseError {
                snippet: input_snippet,
            },
            ParseError::MissingFields => InternalObjectError::MissingFields,
        }
    }
}

pub(crate) fn annotate_with_object_id(id: ObjectId) -> impl Fn(InternalObjectError) -> Error {
    move |internal| match internal {
        InternalObjectError::ExternalError(error) => error,
        InternalObjectError::ObjectTooLarge => Error::ObjectTooLarge(id),
        InternalObjectError::MalformedPackObject => Error::MalformedPackObject(id),
        InternalObjectError::ParseError { snippet } => Error::ObjectParseError { id, snippet },
        InternalObjectError::MissingFields => Error::ObjectMissingRequiredFields(id),
    }
}

impl From<FilesystemError> for InternalObjectError {
    fn from(value: FilesystemError) -> Self {
        Self::ExternalError(value.into())
    }
}

impl From<FilesystemError> for Error {
    fn from(value: FilesystemError) -> Self {
        Self::FileSystem(value)
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
