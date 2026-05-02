//! A module for working with git refs
//!
//! A git ref is a file which points to an object or to another ref.
//!
//! To look up a ref, first construct a [`RefName`] and then use the
//! [`Repo::lookup_ref`] method. A list of all refs in the repository can also
//! be accessed using the [`Repo::ref_names`] method.

use crate::{
    error::{Error, GResult},
    file_system::{Directory, File, FileSystem, FileSystemError},
    object::{Commit, ObjectId, Tree},
    parsing::ParseResult,
    repo::Repo,
};
use accessory::Accessors;
use alloc::vec::Vec;
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take_till},
    character::complete::{char, newline, not_line_ending, space0},
    combinator::{all_consuming, opt},
    multi::many0,
    sequence::{delimited, preceded, terminated},
};

/// The name of a git ref
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum RefName {
    /// The head of a repo, which is called `HEAD` by git.
    Head,
    /// A non-HEAD ref such as `v1.7.0` or `origin/main`.
    Ref(Vec<u8>),
}

impl RefName {
    pub(crate) async fn open_loose_ref<F: FileSystem>(
        &self,
        repo: &Repo<F>,
    ) -> GResult<Option<F::File>> {
        use RefName::*;
        let sub_path = match self {
            Head => {
                return Ok(Some(repo.git_dir.open_file(b"HEAD").await?));
            }
            Ref(path) => path,
        };
        let mut dir = repo.git_dir.open_subdir(b"refs").await?;
        let mut components = sub_path.split(|b| *b == b'/');
        let file_name = components
            .next_back()
            .ok_or_else(|| Error::RefNotFound(self.clone()))?;
        for component in components {
            dir = match dir.open_subdir(component).await {
                Err(FileSystemError::NotFound(_)) => return Ok(None),
                Err(e) => return Err(e.into()),
                Ok(dir) => dir,
            };
        }
        match dir.open_file(file_name).await {
            Err(FileSystemError::NotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
            Ok(file) => Ok(Some(file)),
        }
    }
}

/// The contents of a git ref
#[derive(Accessors, Clone)]
pub struct Ref {
    /// The name of the ref
    #[access(get)]
    name: RefName,

    /// The target of the ref
    ///
    /// Refs can be either direct (pointing to an object) or symbolic (pointing
    /// to another ref).
    #[access(get)]
    target: RefTarget,
}

/// The target of a git ref
///
/// Refs can be either direct (pointing to an object) or symbolic (pointing to
/// another ref).
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RefTarget {
    /// A direct ref, pointing to an object
    Direct(ObjectId),
    /// A symbolic ref, pointing to another ref
    Symbolic(RefName),
}

impl Ref {
    pub(crate) async fn lookup<F: FileSystem>(repo: &Repo<F>, name: &RefName) -> GResult<Ref> {
        let ref_type = {
            if let Some(reference) = lookup_loose_ref(repo, name).await? {
                reference
            } else {
                let mut packed_refs_file = repo.git_dir.open_file(b"packed-refs").await?;
                let packed_refs = read_packed_refs(&mut packed_refs_file).await?;
                if let Some((object_id, _)) = packed_refs
                    .into_iter()
                    .find(|(_, ref_name)| ref_name == name)
                {
                    RefTarget::Direct(object_id)
                } else {
                    return Err(Error::RefNotFound(name.clone()));
                }
            }
        };
        Ok(Self {
            name: name.clone(),
            target: ref_type,
        })
    }

    /// Follow a chain of refs until a direct ref is obtained, and return the
    /// object ID that it points to.
    pub async fn resolve_object_id<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<ObjectId> {
        let mut target: Ref = self.clone();
        while let RefTarget::Symbolic(name) = target.target {
            target = repo.lookup_ref(&name).await?;
        }
        match target.target {
            RefTarget::Symbolic(_) => unreachable!(),
            RefTarget::Direct(oid) => Ok(oid),
        }
    }

    /// Peel the ref to a commit object.
    ///
    /// Returns `None` if the ref does not point to a commit object.
    pub async fn peel_to_commit<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Option<Commit>> {
        let oid = self.resolve_object_id(repo).await?;
        let object = repo.lookup_object(oid).await?;
        object.peel_to_commit(repo).await
    }

    /// Peel the ref to a tree object.
    ///
    /// Returns `None` if the ref does not point to a commit or a tree object.
    pub async fn peel_to_tree<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Option<Tree>> {
        let oid = self.resolve_object_id(repo).await?;
        let object = repo.lookup_object(oid).await?;
        object.peel_to_tree(repo).await
    }
}

impl RefTarget {
    pub(crate) fn parse_loose_ref(content: &[u8]) -> ParseResult<&[u8], Self> {
        all_consuming(terminated(not_line_ending, newline))
            .and_then(alt((
                ObjectId::parse.map(RefTarget::Direct),
                preceded(
                    tag("ref: refs/"),
                    take_till(|_| false)
                        .map(|name: &[u8]| RefTarget::Symbolic(RefName::Ref(name.to_vec()))),
                ),
            )))
            .parse(content)
    }
}

pub(crate) async fn read_packed_refs<F: File>(
    packed_refs_file: &mut F,
) -> GResult<Vec<(ObjectId, RefName)>> {
    let packed_refs_data = packed_refs_file.read_all().await?;
    let parse_one_ref = terminated(
        (
            terminated(ObjectId::parse, char(' ')),
            delimited(
                tag("refs/"),
                not_line_ending.map(|name: &[u8]| RefName::Ref(name.to_vec())),
                newline,
            ),
        ),
        opt(delimited(char('^'), not_line_ending, newline)),
    )
    .map(Some);
    let parse_comment = (space0, char('#'), not_line_ending, opt(newline)).map(|_| None);
    let mut parser = all_consuming(many0(alt((parse_one_ref, parse_comment))));
    let (_, refs) = parser
        .parse(packed_refs_data.as_ref())
        .map_err(|_| Error::MalformedPackedRefs)?;
    Ok(refs.into_iter().flatten().collect())
}

pub(crate) async fn lookup_loose_ref<F: FileSystem>(
    repo: &Repo<F>,
    name: &RefName,
) -> GResult<Option<RefTarget>> {
    let Some(mut ref_file) = name.open_loose_ref(repo).await? else {
        return Ok(None);
    };
    let ref_content = ref_file.read_all().await?;
    let (_, ref_type) =
        RefTarget::parse_loose_ref(&ref_content).map_err(|_| Error::MalformedRef(name.clone()))?;
    Ok(Some(ref_type))
}

#[cfg(test)]
mod test {
    use crate::{
        object::{Object, ObjectId},
        test::helpers::{make_basic_repo, make_packfile_repo},
    };
    use core::matches;
    use futures::executor::block_on;
    use hex_literal::hex;

    use super::*;

    #[test]
    fn resolve_head() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        let head_target = match head.target {
            RefTarget::Direct(_) => panic!(),
            RefTarget::Symbolic(name) => name,
        };
        let head_target = block_on(Ref::lookup(&repo, &head_target)).unwrap();
        assert!(matches!(head_target.target, RefTarget::Direct(_)));
    }

    #[test]
    fn parse_direct_ref() {
        let content = b"6121d0b97779278fcc32cc8a02754e7c588d9c18\n";
        let (_, parsed) = RefTarget::parse_loose_ref(content).unwrap();
        assert_eq!(
            parsed,
            RefTarget::Direct(ObjectId::from_bytes(hex!(
                "6121d0b97779278fcc32cc8a02754e7c588d9c18"
            )))
        );
    }

    #[test]
    fn parse_symbolic_ref() {
        let content = b"ref: refs/heads/main\n";
        let (_, parsed) = RefTarget::parse_loose_ref(content).unwrap();
        assert_eq!(
            parsed,
            RefTarget::Symbolic(RefName::Ref(b"heads/main".to_vec()))
        );
    }

    #[test]
    fn read_thin_packed_ref() {
        let test_repo = make_packfile_repo().unwrap();
        let repo = test_repo.repo();
        let ref_name = RefName::Ref(b"heads/main".to_vec());
        let reference = block_on(repo.lookup_ref(&ref_name)).unwrap();
        let oid = block_on(reference.resolve_object_id(&repo)).unwrap();
        let object = block_on(repo.lookup_object(oid)).unwrap();
        assert!(matches!(object, Object::Commit(_)));
    }

    #[test]
    fn read_fat_packed_ref() {
        let test_repo = make_packfile_repo().unwrap();
        let repo = test_repo.repo();
        let ref_name = RefName::Ref(b"tags/a-fat-tag".to_vec());
        let reference = block_on(repo.lookup_ref(&ref_name)).unwrap();
        let oid = block_on(reference.resolve_object_id(&repo)).unwrap();
        let object = block_on(repo.lookup_object(oid)).unwrap();
        assert!(matches!(object, Object::Tag(_)));
    }

    #[test]
    fn read_stash_packed_ref() {}
}
