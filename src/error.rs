use crate::{
    file_system::{FilesystemError, Offset},
    object::{ObjectId, ObjectType},
    parsing::ParseError,
    reference::RefName,
};
use accessory::Accessors;
use alloc::vec::Vec;
use miniz_oxide::inflate::TINFLStatus;

#[cfg(feature = "serde")]
use serde::Serialize;

pub type GResult<T> = core::result::Result<T, Error>;

#[derive(Debug, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct UnexpectedObjectType {
    #[access(get(cp))]
    pub(crate) id: ObjectId,
    #[access(get(cp))]
    pub(crate) expected: ObjectType,
    #[access(get(cp))]
    pub(crate) received: ObjectType,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum Error {
    FileSystem(#[cfg_attr(feature = "serde", serde(skip))] FilesystemError),
    PathError(#[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))] Vec<u8>),
    LooseObjectDecompressError {
        id: ObjectId,
        #[cfg_attr(feature = "serde", serde(skip))]
        status: TINFLStatus,
    },
    PackObjectDecompressError {
        id: ObjectId,
        error: PackDecompressError,
    },
    FromHexError(#[cfg_attr(feature = "serde", serde(skip))] hex::FromHexError),
    UnsupportedIndexVersion,
    CorruptIndexFile,
    UnsupportedPackVersion,
    CorruptPackFile,
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
    UnexpectedObjectType(UnexpectedObjectType),
}

impl From<UnexpectedObjectType> for Error {
    fn from(value: UnexpectedObjectType) -> Self {
        Self::UnexpectedObjectType(value)
    }
}

impl From<FilesystemError> for Error {
    fn from(value: FilesystemError) -> Self {
        Self::FileSystem(value)
    }
}

impl From<hex::FromHexError> for Error {
    fn from(value: hex::FromHexError) -> Self {
        Self::FromHexError(value)
    }
}

#[derive(Debug)]
pub(crate) enum InternalObjectError {
    ExternalError(Error),
    ObjectTooLarge,
    ParseError { snippet: Vec<u8> },
    MissingFields,
    MalformedPackObject,
    PackObjectDecompressError(PackDecompressError),
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

impl From<FilesystemError> for InternalObjectError {
    fn from(value: FilesystemError) -> Self {
        Self::ExternalError(value.into())
    }
}

pub(crate) fn annotate_with_object_id(id: ObjectId) -> impl Fn(InternalObjectError) -> Error {
    move |internal| match internal {
        InternalObjectError::ExternalError(error) => error,
        InternalObjectError::ObjectTooLarge => Error::ObjectTooLarge(id),
        InternalObjectError::MalformedPackObject => Error::MalformedPackObject(id),
        InternalObjectError::ParseError { snippet } => Error::ObjectParseError { id, snippet },
        InternalObjectError::MissingFields => Error::ObjectMissingRequiredFields(id),
        InternalObjectError::PackObjectDecompressError(error) => {
            Error::PackObjectDecompressError { id, error }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct PackDecompressError {
    pub input_position: usize,
    pub output_position: usize,
    pub pack_offset: Offset,
    #[cfg_attr(feature = "serde", serde(skip))]
    pub status: TINFLStatus,
}
