use crate::{
    error::{Error, GResult, InternalObjectError, annotate_with_object_id},
    file_system::Directory,
    object_store::{
        RawObject,
        lookup::{lookup, lookup_size_type},
    },
    parsing::ParseResult,
    repo::Repo,
};
use accessory::Accessors;
use alloc::{format, vec::Vec};
use chrono::{DateTime, FixedOffset};
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take, take_till, take_until},
    character::complete::{char, hex_digit0, i32, i64, newline, not_line_ending, space1},
    combinator::{all_consuming, not, peek},
    multi::many0,
    sequence::{delimited, preceded, terminated},
};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

mod blob;
mod commit;
mod tag;
mod tree;

pub use crate::object::blob::Blob;
pub use crate::object::commit::Commit;
pub use crate::object::tag::{Tag, TagType};
pub use crate::object::tree::{Tree, TreeEntry, TreeEntryType};
pub use crate::object_store::{ObjectSize, ObjectType};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Accessors)]
pub struct ObjectId {
    #[access(get)]
    pub(crate) id: [u8; 20],
}

impl alloc::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut chars = [0u8; 40];
        hex::encode_to_slice(self.id, &mut chars).unwrap();
        write!(f, "{}", str::from_utf8(&chars).unwrap())
    }
}

impl alloc::fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ObjectId").field(&format!("{self}")).finish()
    }
}

impl ObjectId {
    pub const fn new(id: [u8; 20]) -> Self {
        Self { id }
    }

    pub(crate) fn parse(input: &[u8]) -> ParseResult<&[u8], Self> {
        take(40usize)
            .and_then(all_consuming(hex_digit0))
            .map_res(|hex_str| {
                let mut buf = [0u8; 20];
                hex::decode_to_slice(hex_str, &mut buf)?;
                Ok::<ObjectId, hex::FromHexError>(ObjectId::new(buf))
            })
            .parse(input)
    }

    pub fn from_hex(s: &[u8]) -> Option<Self> {
        let (_, oid) = all_consuming(Self::parse).parse(s).ok()?;
        Some(oid)
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type"))]
#[cfg_attr(feature = "serde", serde(bound = ""))]
pub enum Object<'r, D> {
    Commit(Commit<'r, D>),
    Tree(Tree<'r, D>),
    Tag(Tag<'r, D>),
    Blob(Blob),
}

impl<D> Object<'_, D> {
    pub fn id(&self) -> ObjectId {
        use Object::*;
        match self {
            Commit(c) => c.id(),
            Tree(t) => t.id(),
            Tag(t) => t.id(),
            Blob(b) => b.id(),
        }
    }

    pub fn detach(self) -> Object<'static, ()> {
        use Object::*;
        match self {
            Commit(commit) => Commit(commit.detach()),
            Tree(tree) => Tree(tree.detach()),
            Tag(tag) => Tag(tag.detach()),
            Blob(blob) => Blob(blob),
        }
    }
}

impl<'r, D: Directory> Object<'r, D> {
    pub async fn peel_to_commit(&self) -> GResult<Option<Commit<'r, D>>> {
        use Object::*;
        let mut obj = self.clone();
        loop {
            match obj {
                Commit(c) => return Ok(Some(c)),
                Tag(t) => {
                    let target = t.repo()?.lookup_object(t.target()).await?;
                    obj = target;
                }
                _ => return Ok(None),
            }
        }
    }

    pub async fn peel_to_tree(&self) -> GResult<Option<Tree<'r, D>>> {
        use Object::*;
        let mut obj = self.clone();
        loop {
            match obj {
                Tree(t) => return Ok(Some(t)),
                Commit(c) => {
                    let tree = c.repo()?.lookup_object(c.tree()).await?;
                    obj = tree;
                }
                Tag(t) => {
                    let target = t.repo()?.lookup_object(t.target()).await?;
                    obj = target;
                }
                Blob(_) => return Ok(None),
            }
        }
    }

    pub(crate) async fn lookup(repo: &'r Repo<D>, id: ObjectId) -> GResult<Self> {
        let RawObject {
            object_type,
            body,
            id,
        } = lookup(repo, id)
            .await?
            .ok_or_else(|| Error::MissingObject(id))?;

        let (_, object) = Self::parser(id, object_type, repo)
            .parse(body.as_ref())
            .map_err(|e| match e {
                nom::Err::Incomplete(_) => unreachable!(),
                nom::Err::Error(e) | nom::Err::Failure(e) => InternalObjectError::from(e),
            })
            .map_err(annotate_with_object_id(id))?;
        Ok(object)
    }

    pub(crate) async fn lookup_size_type(
        repo: &'r Repo<D>,
        id: ObjectId,
    ) -> GResult<(ObjectSize, ObjectType)> {
        lookup_size_type(repo, id)
            .await?
            .ok_or_else(|| Error::MissingObject(id))
    }

    pub(crate) fn parser<'a>(
        id: ObjectId,
        object_type: ObjectType,
        repo: &'r Repo<D>,
    ) -> impl Fn(&'a [u8]) -> ParseResult<&'a [u8], Self> {
        move |body: &[u8]| {
            let (_, object) = match object_type {
                ObjectType::Commit => all_consuming(Commit::parser(id, repo))
                    .map(Self::Commit)
                    .parse(body)?,
                ObjectType::Tag => all_consuming(Tag::parser(id, repo))
                    .map(Self::Tag)
                    .parse(body)?,
                ObjectType::Tree => all_consuming(Tree::parser(id, repo))
                    .map(Self::Tree)
                    .parse(body)?,
                ObjectType::Blob => (&[][..], Self::Blob(Blob::new(id, body.to_vec()))),
            };
            Ok((&[][..], object))
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ObjectHeader {
    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    name: Vec<u8>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    value: Vec<u8>,
}

fn parse_object_headers(input: &[u8]) -> ParseResult<&[u8], Vec<ObjectHeader>> {
    let header = (
        delimited(peek(not(newline)), take_till(|c| c == b' '), char(' ')),
        terminated(
            (
                not_line_ending,
                many0(preceded((newline, space1), not_line_ending)),
            ),
            newline,
        ),
    );
    let mut p = terminated(many0(header), newline);
    let (rest, raw_headers) = p.parse(input)?;
    let mut headers: Vec<ObjectHeader> = Vec::new();
    for (name, (first_line, continuation_lines)) in raw_headers {
        let mut full_line = first_line.to_vec();
        for line in continuation_lines {
            full_line.push(b' ');
            full_line.extend_from_slice(line);
        }
        headers.push(ObjectHeader {
            name: name.to_vec(),
            value: full_line,
        });
    }
    Ok((rest, headers))
}

#[allow(clippy::type_complexity)]
fn parse_author_committer_tagger(
    input: &[u8],
) -> ParseResult<&[u8], (&[u8], &[u8], DateTime<FixedOffset>)> {
    (
        terminated(take_until(" <"), tag(" <")),
        terminated(take_until("> "), tag("> ")),
        (
            terminated(i64, char(' ')),
            alt((char('+').map(|_| 1), char('-').map(|_| -1))),
            take(2usize).and_then(all_consuming(i32)),
            take(2usize).and_then(all_consuming(i32)),
        )
            .map_opt(|(timestamp, tz_sign, tz_hour, tz_minute)| {
                let date = DateTime::from_timestamp(timestamp, 0)?;
                let offset = FixedOffset::east_opt(tz_sign * (3600 * tz_hour + 60 * tz_minute))?;
                let author_date = date.with_timezone(&offset);
                Some(author_date)
            }),
    )
        .parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::helpers::{make_basic_repo, make_similar_commits};
    use futures::executor::block_on;

    #[test]
    fn lookup_commit() {
        let test_repo = make_basic_repo().unwrap();
        let commit_id = test_repo.run_git(["rev-parse", "HEAD"]).unwrap();
        let commit_id = ObjectId::from_hex(commit_id.trim_ascii()).unwrap();

        let repo = test_repo.repo();
        let object = block_on(Object::lookup(&repo, commit_id)).unwrap();
        assert_eq!(object.id(), commit_id);
        assert!(matches!(object, Object::Commit(_)));
    }

    #[test]
    fn lookup_packfile_object() {
        let test_repo = make_basic_repo().unwrap();
        make_similar_commits(&test_repo).unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        let oid = block_on(head.resolve_object_id()).unwrap();
        let Object::Commit(commit) = block_on(repo.lookup_object(oid)).unwrap() else {
            panic!()
        };
        let tree_id = commit.tree();
        let Object::Tree(tree) = block_on(repo.lookup_object(tree_id)).unwrap() else {
            panic!()
        };
        assert_eq!(tree.entries().len(), 1 + 26 - 2);
    }

    #[test]
    fn parse_author_committer_line() {
        let example = "an author <an-email-address> 0 +0000";
        parse_author_committer_tagger(example.as_bytes()).unwrap();
    }
}
