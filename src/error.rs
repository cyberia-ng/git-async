//! A module for errors which may occur during the use of `git-async`

use crate::{
    file_system::FileSystemError,
    object::{ObjectId, ObjectType},
    parsing::ParseError,
    reference::RefName,
};
use accessory::Accessors;
use alloc::vec::Vec;
use miniz_oxide::inflate::TINFLStatus;

#[expect(missing_docs)]
pub type GResult<T> = core::result::Result<T, Error>;

#[expect(missing_docs)]
#[derive(Debug, Accessors)]
pub struct UnexpectedObjectType {
    pub id: ObjectId,
    pub expected: ObjectType,
    pub received: ObjectType,
}

#[expect(missing_docs)]
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    FileSystem(FileSystemError),
    PathError(Vec<u8>),
    LooseObjectDecompressError {
        #[expect(missing_docs)]
        id: ObjectId,
        #[expect(missing_docs)]
        status: TINFLStatus,
    },
    PackObjectDecompressError {
        #[expect(missing_docs)]
        id: ObjectId,
        #[expect(missing_docs)]
        status: TINFLStatus,
    },
    FromHexError(hex::FromHexError),
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
        #[expect(missing_docs)]
        id: ObjectId,
        #[expect(missing_docs)]
        snippet: Vec<u8>,
    },
    ObjectMissingRequiredFields(ObjectId),
    MissingObject(ObjectId),
    ObjectTooLarge(ObjectId),
    UnexpectedThinPack,
    NotAnnotatedWithRepo,
    UnexpectedObjectType(UnexpectedObjectType),
    DiffCanceled,
    NotAGitRepository,
}

impl From<UnexpectedObjectType> for Error {
    fn from(value: UnexpectedObjectType) -> Self {
        Self::UnexpectedObjectType(value)
    }
}

impl From<FileSystemError> for Error {
    fn from(value: FileSystemError) -> Self {
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
    PackObjectDecompressError(TINFLStatus),
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

impl From<FileSystemError> for InternalObjectError {
    fn from(value: FileSystemError) -> Self {
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
        InternalObjectError::PackObjectDecompressError(status) => {
            Error::PackObjectDecompressError { id, status }
        }
    }
}
