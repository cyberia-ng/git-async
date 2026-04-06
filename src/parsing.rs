use alloc::vec::Vec;
use core::cmp;

#[derive(Debug)]
pub(crate) enum ParseError {
    Nom {
        _input_snippet: Vec<u8>,
        _kind: nom::error::ErrorKind,
    },
    MissingFields,
}

impl nom::error::ParseError<&[u8]> for ParseError {
    fn from_error_kind(input: &[u8], kind: nom::error::ErrorKind) -> Self {
        Self::Nom {
            _input_snippet: input[0..cmp::min(input.len(), 16)].to_vec(),
            _kind: kind,
        }
    }

    fn append(input: &[u8], kind: nom::error::ErrorKind, _other: Self) -> Self {
        Self::Nom {
            _input_snippet: input[0..cmp::min(input.len(), 16)].to_vec(),
            _kind: kind,
        }
    }
}

impl<E> nom::error::FromExternalError<&[u8], E> for ParseError {
    fn from_external_error(input: &[u8], kind: nom::error::ErrorKind, _e: E) -> Self {
        Self::Nom {
            _input_snippet: input[0..cmp::min(input.len(), 16)].to_vec(),
            _kind: kind,
        }
    }
}

pub(crate) type ParseResult<I, T> = nom::IResult<I, T, ParseError>;
