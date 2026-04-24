use alloc::vec::Vec;
use core::cmp;

#[derive(Debug)]
pub(crate) enum ParseError {
    ParseError { input_snippet: Vec<u8> },
    MissingFields,
}

impl nom::error::ParseError<&[u8]> for ParseError {
    fn from_error_kind(input: &[u8], _kind: nom::error::ErrorKind) -> Self {
        Self::ParseError {
            input_snippet: input[0..cmp::min(input.len(), 16)].to_vec(),
        }
    }

    fn append(input: &[u8], _kind: nom::error::ErrorKind, _other: Self) -> Self {
        Self::ParseError {
            input_snippet: input[0..cmp::min(input.len(), 16)].to_vec(),
        }
    }
}

impl<E> nom::error::FromExternalError<&[u8], E> for ParseError {
    fn from_external_error(input: &[u8], _kind: nom::error::ErrorKind, _e: E) -> Self {
        Self::ParseError {
            input_snippet: input[0..cmp::min(input.len(), 16)].to_vec(),
        }
    }
}

impl From<nom::Err<ParseError>> for ParseError {
    fn from(value: nom::Err<ParseError>) -> Self {
        match value {
            nom::Err::Incomplete(_) => unimplemented!(),
            nom::Err::Error(e) | nom::Err::Failure(e) => e,
        }
    }
}

pub(crate) type ParseResult<I, T> = nom::IResult<I, T, ParseError>;
