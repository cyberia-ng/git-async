use crate::{parsing::ParseResult, subslice_range::SubsliceRange};
use accessory::Accessors;
use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::ops::Range;
use nom::{
    Parser,
    bytes::complete::take_till,
    character::complete::{char, newline},
    combinator::{not, peek},
    multi::many0,
    sequence::{delimited, terminated},
};

/// An arbitrary object header, with a name and a value
#[derive(Clone, PartialEq, Eq, Accessors)]
pub struct ObjectHeader<'a> {
    /// The name of the header
    #[access(get(cp))]
    name: &'a [u8],
    /// The value of the header
    ///
    /// Multi-line values (using wrapped lines) are not decoded.
    #[access(get(cp))]
    value: &'a [u8],
}

#[derive(Clone)]
pub(crate) struct RangeObjectHeader {
    pub(crate) name: Range<usize>,
    pub(crate) value: Range<usize>,
}

impl RangeObjectHeader {
    pub(crate) fn parser(input: &[u8]) -> ParseResult<&[u8], Vec<Self>> {
        let header = (
            delimited(peek(not(newline)), take_till(|c| c == b' '), char(' ')),
            continued_line,
        );
        let mut p = terminated(many0(header), newline);
        let (rest, raw_headers) = p.parse(input)?;
        Ok((
            rest,
            raw_headers
                .into_iter()
                .map(|(name, value)| RangeObjectHeader {
                    name: input.subslice_range_stable(name).unwrap(),
                    value: input.subslice_range_stable(value).unwrap(),
                })
                .collect(),
        ))
    }
}

/// An iterator over additional headers in an object
pub struct ObjectHeaderIter<'a> {
    body: &'a [u8],
    headers: &'a [RangeObjectHeader],
    pos: usize,
}

impl<'a> ObjectHeaderIter<'a> {
    pub(crate) fn new(body: &'a [u8], headers: &'a [RangeObjectHeader]) -> Self {
        Self {
            body,
            headers,
            pos: 0,
        }
    }
}

impl<'a> Iterator for ObjectHeaderIter<'a> {
    type Item = ObjectHeader<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let header = self.headers.get(self.pos)?;
        self.pos += 1;
        Some(ObjectHeader {
            name: &self.body[header.name.clone()],
            value: &self.body[header.value.clone()],
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            self.headers.len() - self.pos,
            Some(self.headers.len() - self.pos),
        )
    }
}

impl FusedIterator for ObjectHeaderIter<'_> {}
impl ExactSizeIterator for ObjectHeaderIter<'_> {}

#[allow(clippy::unnecessary_wraps)]
fn continued_line(input: &[u8]) -> ParseResult<&[u8], &[u8]> {
    let mut slice_pos = input.len();
    for (pos, window) in input.windows(2).enumerate() {
        if window[0] == b'\n' && window[1] != b' ' {
            slice_pos = pos;
            break;
        }
    }
    Ok((&input[(slice_pos + 1)..], &input[..slice_pos]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_continued_line() {
        let data = b"some stuff\n  continuation\nnext line";
        let (rest, parsed) = continued_line.parse(data).unwrap();
        assert_eq!(parsed, b"some stuff\n  continuation");
        assert_eq!(rest, b"next line");
    }
}
