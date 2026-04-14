use crate::{
    error::{Error, GResult},
    object::ObjectId,
    parsing::ParseResult,
    repo::Repo,
};
use accessory::Accessors;
use alloc::vec::Vec;
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take, take_till},
    character::complete::char,
    multi::many,
    sequence::terminated,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TreeEntryType {
    File,
    Executable,
    Symlink,
    Tree,
    Commit,
}

#[derive(Debug, PartialEq, Eq, Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeEntry {
    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    name: Vec<u8>,

    #[access(get(cp))]
    entry_type: TreeEntryType,

    #[access(get(cp))]
    id: ObjectId,
}

#[derive(Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(bound = ""))]
pub struct Tree<'r, D> {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get(ty(&[TreeEntry])))]
    entries: Vec<TreeEntry>,

    #[allow(dead_code)] // TODO Will be useful for diffing
    #[cfg_attr(feature = "serde", serde(skip))]
    repo: Option<&'r Repo<D>>,
}

impl TreeEntry {
    fn parser(input: &[u8]) -> ParseResult<&[u8], Self> {
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
            take(20usize).map(|bytes| ObjectId::new(<[u8; 20]>::try_from(bytes).unwrap())),
        );
        let (rest, (entry_type, name, id)) = p.parse(input)?;
        Ok((
            rest,
            TreeEntry {
                entry_type,
                name: name.to_vec(),
                id,
            },
        ))
    }
}

impl<'r, D> Tree<'r, D> {
    pub fn detach(self) -> Tree<'static, ()> {
        Tree {
            id: self.id,
            entries: self.entries,
            repo: None,
        }
    }

    pub(crate) fn parser<'a>(
        id: ObjectId,
        repo: &'r Repo<D>,
    ) -> impl Fn(&'a [u8]) -> ParseResult<&'a [u8], Self> {
        move |input: &'a [u8]| {
            many(0.., TreeEntry::parser)
                .map(|entries| Tree {
                    id,
                    entries,
                    repo: Some(repo),
                })
                .parse(input)
        }
    }

    #[allow(dead_code)] // TODO Will be useful for diffing
    pub(crate) fn repo(&self) -> GResult<&'r Repo<D>> {
        match self.repo {
            Some(r) => Ok(r),
            None => Err(Error::NotAnnotatedWithRepo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::repo::{TestRepo, TestRepoDirectory};
    use core::iter::zip;
    use hex_literal::hex;

    const ZERO_OID: ObjectId = ObjectId::new([0; 20]);

    fn dummy_repo() -> Repo<TestRepoDirectory> {
        TestRepo::new().unwrap().repo()
    }

    #[test]
    fn parse_tree() {
        let repo = dummy_repo();
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
        let (_, tree) = Tree::parser(ZERO_OID, &repo).parse(&data).unwrap();
        let expected_entries = [
            TreeEntry {
                entry_type: TreeEntryType::Tree,
                id: ObjectId::new(hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb")),
                name: Vec::from(b"a-directory"),
            },
            TreeEntry {
                entry_type: TreeEntryType::File,
                id: ObjectId::new(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                name: Vec::from(b"a-file"),
            },
            TreeEntry {
                entry_type: TreeEntryType::Symlink,
                id: ObjectId::new(hex!("7c35e066a9001b24677ae572214d292cebc55979")),
                name: Vec::from(b"a-symlink"),
            },
            TreeEntry {
                entry_type: TreeEntryType::Executable,
                id: ObjectId::new(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                name: Vec::from(b"an-executable-file"),
            },
            TreeEntry {
                entry_type: TreeEntryType::Commit,
                id: ObjectId::new(hex!("91ca81cfccb6f88a34807e9810bb0be409f32d70")),
                name: Vec::from(b"a-commit"),
            },
        ];
        for (entry, expected) in zip(tree.entries.iter(), expected_entries.iter()) {
            assert_eq!(entry, expected);
        }
    }
}
