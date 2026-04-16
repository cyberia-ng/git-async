use crate::{
    error::{Error, GResult},
    object::{Object, ObjectId},
    parsing::ParseResult,
    repo::Repo,
    traits::{AllGenerics, Noop},
};
use accessory::Accessors;
use alloc::vec::Vec;
use core::fmt::Debug;
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TreeEntryType {
    File,
    Executable,
    Symlink,
    Tree,
    Commit,
}

#[derive(Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(bound = ""))]
pub struct TreeEntry<G: AllGenerics> {
    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    name: Vec<u8>,

    #[access(get(cp))]
    entry_type: TreeEntryType,

    #[access(get(cp))]
    id: ObjectId,

    #[cfg_attr(feature = "serde", serde(skip))]
    repo: Option<Repo<G>>,
}

impl<G: AllGenerics> PartialEq for TreeEntry<G> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.entry_type == other.entry_type && self.id == other.id
    }
}
impl<G: AllGenerics> Eq for TreeEntry<G> {}
impl<G: AllGenerics> Clone for TreeEntry<G> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            entry_type: self.entry_type,
            id: self.id,
            repo: self.repo.clone(),
        }
    }
}
impl<G: AllGenerics> Debug for TreeEntry<G> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TreeEntry")
            .field("name", &self.name)
            .field("entry_type", &self.entry_type)
            .field("id", &self.id)
            .finish()
    }
}

impl<G: AllGenerics> TreeEntry<G> {
    pub(crate) fn repo(&self) -> GResult<&Repo<G>> {
        self.repo
            .as_ref()
            .ok_or_else(|| Error::NotAnnotatedWithRepo)
    }

    pub fn detach(self) -> TreeEntry<Noop> {
        TreeEntry {
            name: self.name,
            entry_type: self.entry_type,
            id: self.id,
            repo: None,
        }
    }

    pub async fn lookup(&self) -> GResult<Option<Object<G>>> {
        if self.entry_type == TreeEntryType::Commit {
            Ok(None)
        } else {
            Ok(Some(self.repo()?.lookup_object(self.id).await?))
        }
    }

    fn parser<'a>(repo: &Repo<G>) -> impl Fn(&'a [u8]) -> ParseResult<&'a [u8], Self> {
        move |input: &'a [u8]| {
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
                    repo: Some(repo.clone()),
                },
            ))
        }
    }
}

#[derive(Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(bound = ""))]
pub struct Tree<G: AllGenerics> {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get(ty(&[TreeEntry<G>])))]
    entries: Vec<TreeEntry<G>>,

    #[cfg_attr(feature = "serde", serde(skip))]
    repo: Option<Repo<G>>,
}

impl<G: AllGenerics> PartialEq for Tree<G> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<G: AllGenerics> Eq for Tree<G> {}
impl<G: AllGenerics> PartialOrd for Tree<G> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<G: AllGenerics> Ord for Tree<G> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}
impl<G: AllGenerics> Clone for Tree<G> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            entries: self.entries.clone(),
            repo: self.repo.clone(),
        }
    }
}

impl<G: AllGenerics> Tree<G> {
    pub fn detach(self) -> Tree<Noop> {
        Tree {
            id: self.id,
            entries: self.entries.into_iter().map(TreeEntry::detach).collect(),
            repo: None,
        }
    }

    pub(crate) fn repo(&self) -> GResult<&Repo<G>> {
        match &self.repo {
            Some(r) => Ok(r),
            None => Err(Error::NotAnnotatedWithRepo),
        }
    }

    pub(crate) fn parser<'a>(
        id: ObjectId,
        repo: &Repo<G>,
    ) -> impl Fn(&'a [u8]) -> ParseResult<&'a [u8], Self> {
        move |input: &'a [u8]| {
            many(0.., TreeEntry::parser(repo))
                .map(|entries| Tree {
                    id,
                    entries,
                    repo: Some(repo.clone()),
                })
                .parse(input)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::iter::zip;
    use hex_literal::hex;

    const ZERO_OID: ObjectId = ObjectId::new([0; 20]);

    fn dummy_repo() -> Repo<Noop> {
        Repo { git_dir: Noop(()) }
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
                repo: None,
            },
            TreeEntry {
                entry_type: TreeEntryType::File,
                id: ObjectId::new(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                name: Vec::from(b"a-file"),
                repo: None,
            },
            TreeEntry {
                entry_type: TreeEntryType::Symlink,
                id: ObjectId::new(hex!("7c35e066a9001b24677ae572214d292cebc55979")),
                name: Vec::from(b"a-symlink"),
                repo: None,
            },
            TreeEntry {
                entry_type: TreeEntryType::Executable,
                id: ObjectId::new(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                name: Vec::from(b"an-executable-file"),
                repo: None,
            },
            TreeEntry {
                entry_type: TreeEntryType::Commit,
                id: ObjectId::new(hex!("91ca81cfccb6f88a34807e9810bb0be409f32d70")),
                name: Vec::from(b"a-commit"),
                repo: None,
            },
        ];
        for (entry, expected) in zip(tree.entries.iter(), expected_entries.iter()) {
            assert_eq!(entry, expected);
        }
    }
}
