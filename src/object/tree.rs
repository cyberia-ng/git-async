use crate::{
    error::GResult,
    file_system::FileSystem,
    object::{Object, ObjectId},
    parsing::{ParseError, ParseResult},
    repo::Repo,
    subslice_range::SubsliceRange,
};
use accessory::Accessors;
use alloc::vec::Vec;
use core::{fmt::Debug, iter::FusedIterator, ops::Range};
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take, take_till},
    character::complete::char,
    combinator::all_consuming,
    multi::many,
    sequence::terminated,
};

/// The type of an entry in a tree
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub enum TreeEntryType {
    /// A non-executable file pointing to a blob
    File,
    /// An executable file pointing to a blob
    Executable,
    /// A symbolic link
    ///
    /// Symbolic links in git are encoded as a tree entry of type symlink
    /// pointing to a blob. The blob's content is the path of the symlink
    /// target.
    Symlink,
    /// A sub-tree, i.e. a subdirectory
    Tree,
    /// A pointer to a commit
    ///
    /// This is used for git submodules.
    Commit,
}

/// An entry in a tree object
///
/// It holds a reference to the data in the [`Tree`].
#[derive(Accessors, Clone, PartialEq, Eq)]
pub struct TreeEntry<'a> {
    /// The name of the tree entry
    #[access(get(cp))]
    name: &'a [u8],

    /// The type of the tree entry
    #[access(get(cp))]
    entry_type: TreeEntryType,

    /// The [`ObjectId`] that the entry points to
    #[access(get(cp))]
    id: ObjectId,
}

impl TreeEntry<'_> {
    /// Look up the target object using the provided [`Repo`].
    ///
    /// Returns `None` if the tree entry is a commit, because in that case it is
    /// a pointer to a commit in an external repository.
    pub async fn lookup<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Option<Object>> {
        if self.entry_type == TreeEntryType::Commit {
            Ok(None)
        } else {
            Ok(Some(repo.lookup_object(self.id).await?))
        }
    }
}

#[derive(Clone)]
struct RangeTreeEntry {
    name: Range<usize>,
    entry_type: TreeEntryType,
    id: ObjectId,
}

impl RangeTreeEntry {
    fn parser(body: &[u8]) -> impl Fn(&[u8]) -> ParseResult<&[u8], Self> {
        |input: &[u8]| {
            let entry_type_parser = alt((
                tag("40000").map(|_| TreeEntryType::Tree),
                tag("100644").map(|_| TreeEntryType::File),
                tag("100755").map(|_| TreeEntryType::Executable),
                tag("120000").map(|_| TreeEntryType::Symlink),
                tag("160000").map(|_| TreeEntryType::Commit),
            ));
            let mut p = (
                terminated(entry_type_parser, char(' ')),
                terminated(take_till(|c| c == b'\0'), char('\0')),
                take(20usize)
                    .map(|bytes| ObjectId::from_bytes(<[u8; 20]>::try_from(bytes).unwrap())),
            );
            let (rest, (entry_type, name, id)) = p.parse(input)?;
            Ok((
                rest,
                RangeTreeEntry {
                    name: body.subslice_range_stable(name).unwrap(),
                    entry_type,
                    id,
                },
            ))
        }
    }
}

/// An iterator over the entries in a tree object
pub struct TreeEntryIter<'a> {
    body: &'a [u8],
    entries: &'a [RangeTreeEntry],
    pos: usize,
}

impl<'a> Iterator for TreeEntryIter<'a> {
    type Item = TreeEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.get(self.pos)?;
        self.pos += 1;
        Some(TreeEntry {
            name: &self.body[entry.name.clone()],
            entry_type: entry.entry_type,
            id: entry.id,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            self.entries.len() - self.pos,
            Some(self.entries.len() - self.pos),
        )
    }
}

impl FusedIterator for TreeEntryIter<'_> {}
impl ExactSizeIterator for TreeEntryIter<'_> {}

/// A tree object
#[derive(Accessors, Clone)]
pub struct Tree {
    /// The [`ObjectId`] of the tree
    #[access(get(cp))]
    id: ObjectId,

    /// The raw data in the object
    #[access(get(ty(&[u8])))]
    body: Vec<u8>,

    entries: Vec<RangeTreeEntry>,
}

impl PartialEq for Tree {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Tree {}
impl PartialOrd for Tree {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Tree {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl Tree {
    /// Get an iterator over the entries in the tree.
    pub fn entries(&self) -> TreeEntryIter<'_> {
        TreeEntryIter {
            body: self.body.as_slice(),
            entries: self.entries.as_slice(),
            pos: 0,
        }
    }

    /// Wrap the [`Tree`] as a generic [`Object`].
    pub fn as_object(self) -> Object {
        Object::Tree(self)
    }

    pub(crate) fn parse(id: ObjectId, body: Vec<u8>) -> Result<Self, ParseError> {
        let (_, entries): (_, Vec<_>) =
            all_consuming(many(0.., RangeTreeEntry::parser(&body))).parse(&body)?;
        Ok(Self { id, body, entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    const ZERO_OID: ObjectId = ObjectId::from_bytes([0; 20]);

    #[test]
    fn parse_tree() {
        let mut data = Vec::new();
        data.extend_from_slice(b"40000 a-directory\0");
        data.extend_from_slice(&hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb"));
        data.extend_from_slice(b"100644 a-file\0");
        data.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        data.extend_from_slice(b"120000 a-symlink\0");
        data.extend_from_slice(&hex!("7c35e066a9001b24677ae572214d292cebc55979"));
        data.extend_from_slice(b"100755 an-executable-file\0");
        data.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        data.extend_from_slice(b"160000 a-commit\0");
        data.extend_from_slice(&hex!("91ca81cfccb6f88a34807e9810bb0be409f32d70"));
        let tree = Tree::parse(ZERO_OID, data).unwrap();
        let entries = tree.entries();
        assert_eq!(entries.len(), 5);
        let expected = [
            (
                TreeEntryType::Tree,
                ObjectId::from_bytes(hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb")),
                b"a-directory".as_slice(),
            ),
            (
                TreeEntryType::File,
                ObjectId::from_bytes(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                b"a-file".as_slice(),
            ),
            (
                TreeEntryType::Symlink,
                ObjectId::from_bytes(hex!("7c35e066a9001b24677ae572214d292cebc55979")),
                b"a-symlink".as_slice(),
            ),
            (
                TreeEntryType::Executable,
                ObjectId::from_bytes(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                b"an-executable-file".as_slice(),
            ),
            (
                TreeEntryType::Commit,
                ObjectId::from_bytes(hex!("91ca81cfccb6f88a34807e9810bb0be409f32d70")),
                b"a-commit".as_slice(),
            ),
        ];
        for (received, (entry_type, id, name)) in entries.zip(expected.into_iter()) {
            assert_eq!(received.entry_type(), entry_type);
            assert_eq!(received.id(), id);
            assert_eq!(received.name(), name);
        }
    }
}
