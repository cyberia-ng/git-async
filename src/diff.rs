use core::convert::Infallible;

use crate::{
    Repo,
    error::{Error, GResult},
    object::{Object, ObjectId, Tree, TreeEntry, TreeEntryType},
    traits::AllGenerics,
};
use accessory::Accessors;
use alloc::format;
use alloc::vec::Vec;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use similar::{TextDiff, TextDiffConfig};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Path(#[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))] Vec<u8>);

impl core::fmt::Debug for Path {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match str::from_utf8(&self.0) {
            Ok(p) => f.debug_tuple("Path").field(&p).finish(),
            Err(_) => f.debug_tuple("Path").field(&"non UTF-8 path").finish(),
        }
    }
}

fn join(path: Option<&Path>, component: &[u8]) -> Path {
    match path {
        Some(p) => {
            let mut out = Vec::with_capacity(p.0.len() + 1 + component.len());
            out.extend_from_slice(&p.0);
            out.push(b'/');
            out.extend_from_slice(component);
            Path(out)
        }
        None => Path(component.to_vec()),
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type"))]
pub enum DiffEntry<Content> {
    LeftOnly {
        path: Path,
        entry_type: TreeEntryType,
        content: Content,
    },
    Both {
        path: Path,
        left_type: TreeEntryType,
        right_type: TreeEntryType,
        content: Content,
    },
    RightOnly {
        path: Path,
        entry_type: TreeEntryType,
        content: Content,
    },
}

impl<Content> DiffEntry<Content> {
    pub fn map_content<T>(&self, fun: impl Fn(&Content) -> T) -> DiffEntry<T> {
        self.map_content_res(|c| Ok::<T, Infallible>(fun(c)))
            .unwrap()
    }

    pub fn map_content_res<T, E>(
        &self,
        fun: impl Fn(&Content) -> Result<T, E>,
    ) -> Result<DiffEntry<T>, E> {
        use DiffEntry::*;
        Ok(match self {
            LeftOnly {
                path,
                entry_type,
                content,
            } => DiffEntry::LeftOnly {
                path: path.clone(),
                entry_type: *entry_type,
                content: fun(content)?,
            },
            Both {
                path,
                left_type,
                right_type,
                content,
            } => DiffEntry::Both {
                path: path.clone(),
                left_type: *left_type,
                right_type: *right_type,
                content: fun(content)?,
            },
            RightOnly {
                path,
                entry_type,
                content,
            } => DiffEntry::RightOnly {
                path: path.clone(),
                entry_type: *entry_type,
                content: fun(content)?,
            },
        })
    }
}

#[derive(Accessors)]
pub struct Diff {
    #[access(get(ty(&[DiffEntry<TextDiff<'static, 'static, [u8]>>])))]
    entries: Vec<DiffEntry<TextDiff<'static, 'static, [u8]>>>,
}

#[derive(Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(bound = ""))]
pub struct TreeDiff<G: AllGenerics> {
    #[access(get(ty(&[DiffEntry<(ObjectId, ObjectId)>])))]
    entries: Vec<DiffEntry<(ObjectId, ObjectId)>>,

    #[cfg_attr(feature = "serde", serde(skip))]
    repo: Option<Repo<G>>,
}

impl<G: AllGenerics> TreeDiff<G> {
    pub(crate) fn repo(&self) -> GResult<&Repo<G>> {
        self.repo
            .as_ref()
            .ok_or_else(|| Error::NotAnnotatedWithRepo)
    }

    pub async fn new(left: &Tree<G>, right: &Tree<G>) -> GResult<Self> {
        let repo = left.repo()?;
        if left.id() == right.id() {
            return Ok(Self {
                entries: Vec::new(),
                repo: Some(repo.clone()),
            });
        }
        let mut out: Vec<DiffEntry<(ObjectId, ObjectId)>> = Vec::new();
        #[allow(clippy::type_complexity)]
        let mut stack: Vec<(Option<Path>, Option<Tree<G>>, Option<Tree<G>>)> = Vec::new();
        stack.push((None, Some(left.clone()), Some(right.clone())));

        while let Some((parent_path, left, right)) = stack.pop() {
            // Loop invariants:
            // - one of left or right is Some()
            // - left and right have different IDs
            debug_assert!(left.is_some() || right.is_some());
            debug_assert!(left.as_ref().map(|t| t.id()) != right.as_ref().map(|t| t.id()));
            let (left, right) = match (left, right) {
                (Some(left), Some(right)) => (left, right),
                (Some(left), None) => {
                    for entry in left.entries() {
                        let path = join(parent_path.as_ref(), entry.name());
                        if entry.entry_type() == TreeEntryType::Tree {
                            let tree = tree(repo, entry.id()).await?;
                            stack.push((Some(path), None, Some(tree)));
                        } else {
                            out.push(DiffEntry::LeftOnly {
                                path,
                                entry_type: entry.entry_type(),
                                content: (entry.id(), ObjectId::zero()),
                            });
                        }
                    }
                    continue;
                }
                (None, Some(right)) => {
                    for entry in right.entries() {
                        let path = join(parent_path.as_ref(), entry.name());
                        if entry.entry_type() == TreeEntryType::Tree {
                            let tree = tree(repo, entry.id()).await?;
                            stack.push((Some(path), None, Some(tree)));
                        } else {
                            out.push(DiffEntry::RightOnly {
                                path,
                                entry_type: entry.entry_type(),
                                content: (ObjectId::zero(), entry.id()),
                            });
                        }
                    }
                    continue;
                }
                (None, None) => unreachable!(),
            };

            let mut left_only: Vec<&TreeEntry<G>> = Vec::new();
            let mut right_only: Vec<&TreeEntry<G>> = Vec::new();
            let mut both: Vec<(&TreeEntry<G>, &TreeEntry<G>)> = Vec::new();
            for left_entry in left.entries() {
                let right_entry = right
                    .entries()
                    .iter()
                    .find(|e| e.name() == left_entry.name());
                match right_entry {
                    Some(e) => both.push((left_entry, e)),
                    None => left_only.push(left_entry),
                }
            }
            for right_entry in right.entries() {
                if both
                    .iter()
                    .find(|(_, e)| e.name() == right_entry.name())
                    .is_none()
                {
                    right_only.push(right_entry);
                }
            }
            for entry in left_only {
                let path = join(parent_path.as_ref(), entry.name());
                if entry.entry_type() == TreeEntryType::Tree {
                    let left_tree = tree(repo, entry.id()).await?;
                    stack.push((Some(path), Some(left_tree), None));
                } else {
                    out.push(DiffEntry::LeftOnly {
                        path,
                        entry_type: entry.entry_type(),
                        content: (entry.id(), ObjectId::zero()),
                    });
                }
            }
            for entry in right_only {
                let path = join(parent_path.as_ref(), entry.name());
                if entry.entry_type() == TreeEntryType::Tree {
                    let right_tree = tree(repo, entry.id()).await?;
                    stack.push((Some(path), None, Some(right_tree)));
                } else {
                    out.push(DiffEntry::RightOnly {
                        path,
                        entry_type: entry.entry_type(),
                        content: (ObjectId::zero(), entry.id()),
                    });
                }
            }
            for (left, right) in both {
                if left.id() == right.id() {
                    continue;
                }
                let name = left.name();
                match (left.entry_type(), right.entry_type()) {
                    (TreeEntryType::Tree, TreeEntryType::Tree) => {
                        let left = tree(repo, left.id()).await?;
                        let right = tree(repo, right.id()).await?;
                        let path = join(parent_path.as_ref(), name);
                        stack.push((Some(path), Some(left), Some(right)));
                    }
                    (TreeEntryType::Tree, _) => {
                        let path = join(parent_path.as_ref(), name);
                        out.push(DiffEntry::RightOnly {
                            path: path.clone(),
                            entry_type: right.entry_type(),
                            content: (ObjectId::zero(), right.id()),
                        });
                        let left_tree = tree(repo, left.id()).await?;
                        stack.push((Some(path), Some(left_tree), None));
                    }
                    (_, TreeEntryType::Tree) => {
                        let path = join(parent_path.as_ref(), name);
                        out.push(DiffEntry::LeftOnly {
                            path: path.clone(),
                            entry_type: left.entry_type(),
                            content: (left.id(), ObjectId::zero()),
                        });
                        let right_tree = tree(repo, right.id()).await?;
                        stack.push((Some(path), None, Some(right_tree)));
                    }
                    _ => {
                        out.push(DiffEntry::Both {
                            path: join(parent_path.as_ref(), name),
                            left_type: left.entry_type(),
                            right_type: right.entry_type(),
                            content: (left.id(), right.id()),
                        });
                    }
                }
            }
        }
        Ok(Self {
            entries: out,
            repo: Some(repo.clone()),
        })
    }

    pub async fn to_text_diff(&self, config: TextDiffConfig) -> GResult<Diff> {
        let mut out: Vec<_> = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let entry = entry.resolve(self.repo()?, config.clone()).await?;
            out.push(entry)
        }
        Ok(Diff { entries: out })
    }
}

async fn tree<G: AllGenerics>(repo: &Repo<G>, id: ObjectId) -> GResult<Tree<G>> {
    repo.lookup_object(id)
        .await?
        .peel_to_tree()
        .await?
        .ok_or_else(|| Error::MalformedObject(id))
}

impl DiffEntry<(ObjectId, ObjectId)> {
    pub(crate) async fn resolve<G: AllGenerics>(
        &self,
        repo: &Repo<G>,
        config: TextDiffConfig,
    ) -> GResult<DiffEntry<TextDiff<'static, 'static, [u8]>>> {
        match self {
            DiffEntry::LeftOnly {
                path,
                entry_type,
                content: (id, _),
            } => {
                let body = read_leaf(repo, *entry_type, *id).await?;
                Ok(DiffEntry::LeftOnly {
                    path: path.clone(),
                    entry_type: *entry_type,
                    content: config.diff_lines(body, Vec::new()),
                })
            }
            DiffEntry::RightOnly {
                path,
                entry_type,
                content: (_, id),
            } => {
                let body = read_leaf(repo, *entry_type, *id).await?;
                Ok(DiffEntry::RightOnly {
                    path: path.clone(),
                    entry_type: *entry_type,
                    content: config.diff_lines(Vec::new(), body),
                })
            }
            DiffEntry::Both {
                path,
                left_type,
                right_type,
                content: (left_id, right_id),
            } => {
                let left_body = read_leaf(repo, *left_type, *left_id).await?;
                let right_body = read_leaf(repo, *right_type, *right_id).await?;
                let diff = config.diff_lines(left_body, right_body);
                Ok(DiffEntry::Both {
                    path: path.clone(),
                    left_type: *left_type,
                    right_type: *right_type,
                    content: diff,
                })
            }
        }
    }
}

async fn read_leaf<G: AllGenerics>(
    repo: &Repo<G>,
    entry_type: TreeEntryType,
    id: ObjectId,
) -> GResult<Vec<u8>> {
    debug_assert!(entry_type != TreeEntryType::Tree);
    if entry_type == TreeEntryType::Commit {
        let s = format!("{}", id);
        return Ok(s.into_bytes());
    }
    let object = repo.lookup_object(id).await?;
    if let Object::Blob(b) = object {
        return Ok(b.data_owned());
    }
    unreachable!("Tree entry resolved object was not a blob")
}

#[cfg(test)]
mod tests {
    use crate::{
        Repo,
        reference::RefName,
        test::{
            helpers::{make_basic_repo, make_file},
            impls::TestGenerics,
        },
    };
    use futures::executor::block_on;
    use std::{
        collections::BTreeSet,
        fs::{create_dir, remove_file},
        io::Write,
        path::PathBuf,
    };

    use super::*;

    fn head_tree(repo: &Repo<TestGenerics>) -> Tree<TestGenerics> {
        let head = block_on(repo.lookup_ref(&RefName::Head)).unwrap();
        block_on(head.peel_to_tree()).unwrap().unwrap()
    }

    #[test]
    fn diff_same() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        let tree = head_tree(&repo);
        assert!(
            block_on(TreeDiff::new(&tree, &tree))
                .unwrap()
                .entries()
                .is_empty()
        )
    }

    #[test]
    fn basic_root_diff() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        let mut file_a = make_file(&test_repo, "a").unwrap();
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit("a commit", "a user", "an-email", "2000-01-01T00:00:00Z")
            .unwrap();
        let before = head_tree(&repo);
        file_a.write_all(b"some data").unwrap();
        file_a.flush().unwrap();
        let mut file_b = make_file(&test_repo, "b").unwrap();
        file_b.write_all(b"some more data").unwrap();
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit("a commit", "a user", "an-email", "2000-01-01T00:00:00Z")
            .unwrap();
        let after = head_tree(&repo);
        let the_diff = block_on(TreeDiff::new(&before, &after))
            .unwrap()
            .entries()
            .iter()
            .map(Clone::clone)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            the_diff,
            vec![
                DiffEntry::Both {
                    path: Path(b"a".to_vec()),
                    left_type: TreeEntryType::File,
                    right_type: TreeEntryType::File,
                    content: (
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                        ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58").unwrap(),
                    ),
                },
                DiffEntry::RightOnly {
                    path: Path(b"b".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::zero(),
                        ObjectId::from_hex(b"dfa37ec69ffae3abcf7efbb386226cb84b510fa8").unwrap()
                    )
                }
            ]
            .into_iter()
            .collect()
        );
        let the_diff = block_on(TreeDiff::new(&after, &before))
            .unwrap()
            .entries()
            .iter()
            .map(Clone::clone)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            the_diff,
            vec![
                DiffEntry::Both {
                    path: Path(b"a".to_vec()),
                    left_type: TreeEntryType::File,
                    right_type: TreeEntryType::File,
                    content: (
                        ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58").unwrap(),
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                    ),
                },
                DiffEntry::LeftOnly {
                    path: Path(b"b".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::from_hex(b"dfa37ec69ffae3abcf7efbb386226cb84b510fa8").unwrap(),
                        ObjectId::zero()
                    )
                }
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn basic_subtree_diff() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        create_dir(test_repo.location.path().join("dir")).unwrap();
        let mut file_a = make_file(&test_repo, PathBuf::from("dir").join("a")).unwrap();
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit("a commit", "a user", "an-email", "2000-01-01T00:00:00Z")
            .unwrap();
        let before = head_tree(&repo);
        file_a.write_all(b"some data").unwrap();
        file_a.flush().unwrap();
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit("a commit", "a user", "an-email", "2000-01-01T00:00:00Z")
            .unwrap();
        let after = head_tree(&repo);
        let the_diff = block_on(TreeDiff::new(&before, &after))
            .unwrap()
            .entries()
            .iter()
            .map(Clone::clone)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            the_diff,
            vec![DiffEntry::Both {
                path: Path(b"dir/a".to_vec()),
                left_type: TreeEntryType::File,
                right_type: TreeEntryType::File,
                content: (
                    ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                    ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58").unwrap(),
                )
            },]
            .into_iter()
            .collect()
        );
        let the_diff = block_on(TreeDiff::new(&after, &before))
            .unwrap()
            .entries()
            .iter()
            .map(Clone::clone)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            the_diff,
            vec![DiffEntry::Both {
                path: Path(b"dir/a".to_vec()),
                left_type: TreeEntryType::File,
                right_type: TreeEntryType::File,
                content: (
                    ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58").unwrap(),
                    ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                ),
            },]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn complex_subtree_diff() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        make_file(&test_repo, "a").unwrap();
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit("a commit", "a user", "an-email", "2000-01-01T00:00:00Z")
            .unwrap();
        let before = head_tree(&repo);
        remove_file(test_repo.location.path().join("a")).unwrap();
        create_dir(test_repo.location.path().join("a")).unwrap();
        make_file(&test_repo, PathBuf::from("a").join("b")).unwrap();
        create_dir(test_repo.location.path().join("dir")).unwrap();
        make_file(&test_repo, PathBuf::from("dir").join("c")).unwrap();
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit("a commit", "a user", "an-email", "2000-01-01T00:00:00Z")
            .unwrap();
        let after = head_tree(&repo);
        let the_diff = block_on(TreeDiff::new(&before, &after))
            .unwrap()
            .entries()
            .iter()
            .map(Clone::clone)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            the_diff,
            vec![
                DiffEntry::RightOnly {
                    path: Path(b"a/b".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::zero(),
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                    )
                },
                DiffEntry::LeftOnly {
                    path: Path(b"a".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                        ObjectId::zero()
                    )
                },
                DiffEntry::RightOnly {
                    path: Path(b"dir/c".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::zero(),
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                    )
                },
            ]
            .into_iter()
            .collect()
        );
        let the_diff = block_on(TreeDiff::new(&after, &before))
            .unwrap()
            .entries()
            .iter()
            .map(Clone::clone)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            the_diff,
            vec![
                DiffEntry::LeftOnly {
                    path: Path(b"a/b".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                        ObjectId::zero()
                    )
                },
                DiffEntry::RightOnly {
                    path: Path(b"a".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::zero(),
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                    )
                },
                DiffEntry::LeftOnly {
                    path: Path(b"dir/c".to_vec()),
                    entry_type: TreeEntryType::File,
                    content: (
                        ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                        ObjectId::zero()
                    )
                },
            ]
            .into_iter()
            .collect()
        );
    }
}
