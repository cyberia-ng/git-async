//! A module for working with git objects
//!
//! This module contains data types for all git objects. Objects are acquired
//! from a [`Repo`] by looking them up using their [`ObjectId`], or from one of
//! the `lookup_*` family of methods on existing objects.

use crate::{
    error::{Error, GResult, InternalObjectError, UnexpectedObjectType, annotate_with_object_id},
    file_system::FileSystem,
    object_store::{
        RawObject,
        lookup::{lookup, lookup_size_type},
    },
    parsing::ParseResult,
    repo::Repo,
};
use accessory::Accessors;
use alloc::format;
use chrono::{DateTime, FixedOffset};
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take, take_until},
    character::complete::{char, hex_digit0, i32, i64},
    combinator::all_consuming,
    sequence::terminated,
};

mod blob;
mod commit;
mod header;
mod tag;
mod tree;

pub use crate::object::blob::Blob;
pub use crate::object::commit::Commit;
pub use crate::object::header::{ObjectHeader, ObjectHeaderIter};
pub use crate::object::tag::Tag;
pub use crate::object::tree::{Tree, TreeEntry, TreeEntryIter, TreeEntryType};
pub use crate::object_store::{ObjectSize, ObjectType};

/// The ID of a git object
///
/// `git-async` only supports SHA-1 repositories, so this is always 20 bytes or
/// 40 hex characters
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Accessors)]
pub struct ObjectId {
    /// The object ID as an array of bytes
    #[access(get)]
    pub(crate) bytes: [u8; 20],
}

impl alloc::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut chars = [0u8; 40];
        hex::encode_to_slice(self.bytes, &mut chars).unwrap();
        write!(f, "{}", str::from_utf8(&chars).unwrap())
    }
}

impl alloc::fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ObjectId").field(&format!("{self}")).finish()
    }
}

impl ObjectId {
    /// Construct an [`ObjectId`] from an array of bytes.
    pub const fn from_bytes(id: [u8; 20]) -> Self {
        Self { bytes: id }
    }

    /// Construct an [`ObjectId`] from a hex (byte)string.
    ///
    /// Returns `None` if the provided string was not 40 hexadecimal characters.
    pub fn from_hex(s: &[u8]) -> Option<Self> {
        let (_, oid) = all_consuming(Self::parse).parse(s).ok()?;
        Some(oid)
    }

    pub(crate) const fn zero() -> Self {
        Self { bytes: [0u8; 20] }
    }

    pub(crate) fn parse(input: &[u8]) -> ParseResult<&[u8], Self> {
        take(40usize)
            .and_then(all_consuming(hex_digit0))
            .map_res(|hex_str| {
                let mut buf = [0u8; 20];
                hex::decode_to_slice(hex_str, &mut buf)?;
                Ok::<ObjectId, hex::FromHexError>(ObjectId::from_bytes(buf))
            })
            .parse(input)
    }
}

/// A git object
///
/// This type encapsulates the four possible types of git object.
#[derive(Clone)]
pub enum Object {
    #[expect(missing_docs)]
    Commit(Commit),
    #[expect(missing_docs)]
    Tree(Tree),
    #[expect(missing_docs)]
    Tag(Tag),
    #[expect(missing_docs)]
    Blob(Blob),
}

impl Object {
    /// The ID of the object
    pub fn id(&self) -> ObjectId {
        use Object::*;
        match self {
            Commit(c) => c.id(),
            Tree(t) => t.id(),
            Tag(t) => t.id(),
            Blob(b) => b.id(),
        }
    }

    /// Get the object type as a plain (fieldless) enum.
    pub fn object_type(&self) -> ObjectType {
        use Object::*;
        match self {
            Commit(_) => ObjectType::Commit,
            Tree(_) => ObjectType::Tree,
            Tag(_) => ObjectType::Tag,
            Blob(_) => ObjectType::Blob,
        }
    }

    /// Coerce the object to a [`Commit`].
    ///
    /// Returns `Err` if the object was not a commit.
    pub fn commit(self) -> Result<Commit, UnexpectedObjectType> {
        use Object::*;
        match self {
            Commit(c) => Ok(c),
            _ => Err(UnexpectedObjectType {
                id: self.id(),
                expected: ObjectType::Commit,
                received: self.object_type(),
            }),
        }
    }

    /// Coerce the object to a [`Tag`].
    ///
    /// Returns `Err` if the object was not a tag.
    pub fn tag(self) -> Result<Tag, UnexpectedObjectType> {
        use Object::*;
        match self {
            Tag(t) => Ok(t),
            _ => Err(UnexpectedObjectType {
                id: self.id(),
                expected: ObjectType::Tag,
                received: self.object_type(),
            }),
        }
    }

    /// Coerce the object to a [`Tree`]
    ///
    /// Returns `Err` if the object was not a tree.
    pub fn tree(self) -> Result<Tree, UnexpectedObjectType> {
        use Object::*;
        match self {
            Tree(t) => Ok(t),
            _ => Err(UnexpectedObjectType {
                id: self.id(),
                expected: ObjectType::Tree,
                received: self.object_type(),
            }),
        }
    }

    /// Coerce the object to a [`Blob`]
    ///
    /// Returns `Err` if the object was not a blob.
    pub fn blob(self) -> Result<Blob, UnexpectedObjectType> {
        use Object::*;
        match self {
            Blob(b) => Ok(b),
            _ => Err(UnexpectedObjectType {
                id: self.id(),
                expected: ObjectType::Blob,
                received: self.object_type(),
            }),
        }
    }

    /// Peel the object to a [`Commit`], if possible.
    pub async fn peel_to_commit<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Option<Commit>> {
        use Object::*;
        let mut obj: Object = self.clone();
        loop {
            match obj {
                Commit(c) => return Ok(Some(c)),
                Tag(t) => {
                    let target = repo.lookup_object(t.target()).await?;
                    obj = target;
                }
                _ => return Ok(None),
            }
        }
    }

    /// Peel the object to a [`Tree`], if possible.
    pub async fn peel_to_tree<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Option<Tree>> {
        use Object::*;
        let mut obj: Object = self.clone();
        loop {
            match obj {
                Tree(t) => return Ok(Some(t)),
                Commit(c) => {
                    let tree = repo.lookup_object(c.tree()).await?;
                    obj = tree;
                }
                Tag(t) => {
                    let target = repo.lookup_object(t.target()).await?;
                    obj = target;
                }
                Blob(_) => return Ok(None),
            }
        }
    }

    pub(crate) async fn lookup<F: FileSystem>(repo: &Repo<F>, id: ObjectId) -> GResult<Self> {
        let RawObject { object_type, body } = lookup(repo, id)
            .await?
            .ok_or_else(|| Error::MissingObject(id))?;

        let object = match object_type {
            ObjectType::Commit => Object::Commit(
                Commit::parse(id, body)
                    .map_err(InternalObjectError::from)
                    .map_err(annotate_with_object_id(id))?,
            ),
            ObjectType::Tag => Object::Tag(
                Tag::parse(id, body)
                    .map_err(InternalObjectError::from)
                    .map_err(annotate_with_object_id(id))?,
            ),
            ObjectType::Blob => Object::Blob(Blob::new(id, body)),
            ObjectType::Tree => Object::Tree(
                Tree::parse(id, body)
                    .map_err(InternalObjectError::from)
                    .map_err(annotate_with_object_id(id))?,
            ),
        };

        Ok(object)
    }

    pub(crate) async fn lookup_size_type<F: FileSystem>(
        repo: &Repo<F>,
        id: ObjectId,
    ) -> GResult<(ObjectSize, ObjectType)> {
        lookup_size_type(repo, id)
            .await?
            .ok_or_else(|| Error::MissingObject(id))
    }
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
        let oid = block_on(head.resolve_object_id(&repo)).unwrap();
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
