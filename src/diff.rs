use crate::{
    Repo,
    error::{Error, GResult},
    file_system::Directory,
    object::{ObjectId, Tree, TreeEntry, TreeEntryType},
};
use alloc::vec::Vec;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Path(Vec<u8>);

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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiffEntry {
    LeftOnly {
        path: Path,
        id: ObjectId,
    },
    Both {
        path: Path,
        left_id: ObjectId,
        right_id: ObjectId,
    },
    RightOnly {
        path: Path,
        id: ObjectId,
    },
}

async fn tree<D: Directory>(repo: &Repo<D>, id: ObjectId) -> GResult<Tree<D>> {
    repo.lookup_object(id)
        .await?
        .peel_to_tree()
        .await?
        .ok_or_else(|| Error::MalformedObject(id))
}

pub async fn diff<D: Directory>(left: &Tree<D>, right: &Tree<D>) -> GResult<Vec<DiffEntry>> {
    if left.id() == right.id() {
        return Ok(Vec::new());
    }
    let repo = left.repo()?;
    let mut out: Vec<DiffEntry> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut stack: Vec<(Option<Path>, Option<Tree<D>>, Option<Tree<D>>)> = Vec::new();
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
                            id: entry.id(),
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
                            id: entry.id(),
                        });
                    }
                }
                continue;
            }
            (None, None) => unreachable!(),
        };

        let mut left_only: Vec<&TreeEntry<D>> = Vec::new();
        let mut right_only: Vec<&TreeEntry<D>> = Vec::new();
        let mut both: Vec<(&TreeEntry<D>, &TreeEntry<D>)> = Vec::new();
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
                    id: entry.id(),
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
                    id: entry.id(),
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
                        id: right.id(),
                    });
                    let left_tree = tree(repo, left.id()).await?;
                    stack.push((Some(path), Some(left_tree), None));
                }
                (_, TreeEntryType::Tree) => {
                    let path = join(parent_path.as_ref(), name);
                    out.push(DiffEntry::LeftOnly {
                        path: path.clone(),
                        id: left.id(),
                    });
                    let right_tree = tree(repo, right.id()).await?;
                    stack.push((Some(path), None, Some(right_tree)));
                }
                _ => {
                    out.push(DiffEntry::Both {
                        path: join(parent_path.as_ref(), name),
                        left_id: left.id(),
                        right_id: right.id(),
                    });
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::{
        Repo,
        reference::RefName,
        test::{
            helpers::{make_basic_repo, make_file},
            repo::TestRepoDirectory,
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

    fn head_tree(repo: &Repo<TestRepoDirectory>) -> Tree<TestRepoDirectory> {
        let head = block_on(repo.lookup_ref(&RefName::Head)).unwrap();
        block_on(head.peel_to_tree()).unwrap().unwrap()
    }

    #[test]
    fn diff_same() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        let tree = head_tree(&repo);
        assert!(block_on(diff(&tree, &tree)).unwrap().is_empty())
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
        let the_diff = block_on(diff(&before, &after)).unwrap();
        assert_eq!(
            the_diff.into_iter().collect::<BTreeSet<_>>(),
            vec![
                DiffEntry::Both {
                    path: Path(b"a".to_vec()),
                    left_id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")
                        .unwrap(),
                    right_id: ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58")
                        .unwrap(),
                },
                DiffEntry::RightOnly {
                    path: Path(b"b".to_vec()),
                    id: ObjectId::from_hex(b"dfa37ec69ffae3abcf7efbb386226cb84b510fa8").unwrap()
                }
            ]
            .into_iter()
            .collect()
        );
        let the_diff = block_on(diff(&after, &before)).unwrap();
        assert_eq!(
            the_diff.into_iter().collect::<BTreeSet<_>>(),
            vec![
                DiffEntry::Both {
                    path: Path(b"a".to_vec()),
                    left_id: ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58")
                        .unwrap(),
                    right_id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")
                        .unwrap(),
                },
                DiffEntry::LeftOnly {
                    path: Path(b"b".to_vec()),
                    id: ObjectId::from_hex(b"dfa37ec69ffae3abcf7efbb386226cb84b510fa8").unwrap()
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
        let the_diff = block_on(diff(&before, &after)).unwrap();
        assert_eq!(
            the_diff.into_iter().collect::<BTreeSet<_>>(),
            vec![DiffEntry::Both {
                path: Path(b"dir/a".to_vec()),
                left_id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                right_id: ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58").unwrap(),
            },]
            .into_iter()
            .collect()
        );
        let the_diff = block_on(diff(&after, &before)).unwrap();
        assert_eq!(
            the_diff.into_iter().collect::<BTreeSet<_>>(),
            vec![DiffEntry::Both {
                path: Path(b"dir/a".to_vec()),
                left_id: ObjectId::from_hex(b"7c0646bfd53c1f0ed45ffd81563f30017717ca58").unwrap(),
                right_id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
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
        let the_diff = block_on(diff(&before, &after)).unwrap();
        assert_eq!(
            the_diff.into_iter().collect::<BTreeSet<_>>(),
            vec![
                DiffEntry::RightOnly {
                    path: Path(b"a/b".to_vec()),
                    id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                },
                DiffEntry::LeftOnly {
                    path: Path(b"a".to_vec()),
                    id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                },
                DiffEntry::RightOnly {
                    path: Path(b"dir/c".to_vec()),
                    id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                },
            ]
            .into_iter()
            .collect()
        );
        let the_diff = block_on(diff(&after, &before)).unwrap();
        assert_eq!(
            the_diff.into_iter().collect::<BTreeSet<_>>(),
            vec![
                DiffEntry::LeftOnly {
                    path: Path(b"a/b".to_vec()),
                    id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                },
                DiffEntry::RightOnly {
                    path: Path(b"a".to_vec()),
                    id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                },
                DiffEntry::LeftOnly {
                    path: Path(b"dir/c".to_vec()),
                    id: ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
                },
            ]
            .into_iter()
            .collect()
        );
    }
}
