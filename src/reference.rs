use crate::{
    directory::{Directory, DirectoryError, File},
    error::{Error, GResult},
    object::{Object, ObjectId},
    parsing::ParseResult,
    repo::Repo,
};
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

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", content = "value"))]
pub enum RefName {
    Branch(#[cfg_attr(feature = "serde", serde(with = "serde_bytes"))] Vec<u8>),
    Tag(#[cfg_attr(feature = "serde", serde(with = "serde_bytes"))] Vec<u8>),
    Remote(#[cfg_attr(feature = "serde", serde(with = "serde_bytes"))] Vec<u8>),
    Head,
}

impl RefName {
    pub(crate) async fn open_file<D: Directory>(&self, repo: &Repo<D>) -> GResult<Option<D::File>> {
        use RefName::*;
        let sub_path = match self {
            Head => {
                return Ok(Some(repo.git_dir.open_file(b"HEAD").await?));
            }
            Branch(sub_path) => sub_path,
            Tag(sub_path) => sub_path,
            Remote(sub_path) => sub_path,
        };
        let mut dir = repo.git_dir.open_subdir(b"refs").await?;
        dir = match self {
            Branch(_) => dir.open_subdir(b"heads").await?,
            Tag(_) => dir.open_subdir(b"tags").await?,
            Remote(_) => dir.open_subdir(b"remotes").await?,
            Head => unreachable!(),
        };
        let mut components = sub_path.split(|b| *b == b'/');
        let file_name = components
            .next_back()
            .ok_or_else(|| Error::RefNotFound(self.clone()))?;
        for component in components {
            dir = match dir.open_subdir(component).await {
                Err(DirectoryError::NotFound(_)) => return Ok(None),
                Err(e) => return Err(e.into()),
                Ok(dir) => dir,
            };
        }
        match dir.open_file(file_name).await {
            Err(DirectoryError::NotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
            Ok(file) => Ok(Some(file)),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", content = "value"))]
pub enum Ref {
    Direct(ObjectId),
    Symbolic(RefName),
}

impl Ref {
    pub async fn lookup<D: Directory>(repo: &Repo<D>, name: &RefName) -> GResult<Ref> {
        if let Some(reference) = lookup_loose_ref(repo, name).await? {
            return Ok(reference);
        }
        let mut packed_refs_file = repo.git_dir.open_file(b"packed-refs").await?;
        let packed_refs = read_packed_refs(&mut packed_refs_file).await?;
        if let Some((object_id, _)) = packed_refs
            .into_iter()
            .find(|(_, ref_name)| ref_name == name)
        {
            return Ok(Ref::Direct(object_id));
        }
        Err(Error::RefNotFound(name.clone()))
    }

    pub async fn resolve_to_object<D: Directory>(&self, repo: &Repo<D>) -> GResult<Object> {
        let mut target = self.clone();
        while let Ref::Symbolic(name) = target {
            target = Ref::lookup(repo, &name).await?;
        }
        let oid = match target {
            Ref::Symbolic(_) => unreachable!(),
            Ref::Direct(oid) => oid,
        };
        Object::lookup(repo, oid).await
    }

    pub(crate) fn parse_loose_ref(content: &[u8]) -> ParseResult<&[u8], Self> {
        all_consuming(terminated(not_line_ending, newline))
            .and_then(alt((
                ObjectId::parse.map(Ref::Direct),
                preceded(
                    tag("ref: refs/"),
                    alt((
                        preceded(tag("heads/"), take_till(|_| false))
                            .map(|name: &[u8]| Ref::Symbolic(RefName::Branch(name.to_vec()))),
                        preceded(tag("tags/"), take_till(|_| false))
                            .map(|name: &[u8]| Ref::Symbolic(RefName::Tag(name.to_vec()))),
                        preceded(tag("remotes/"), take_till(|_| false))
                            .map(|name: &[u8]| Ref::Symbolic(RefName::Remote(name.to_vec()))),
                    )),
                ),
            )))
            .parse(content)
    }
}

async fn read_packed_refs<F: File>(packed_refs_file: &mut F) -> GResult<Vec<(ObjectId, RefName)>> {
    let packed_refs_data = packed_refs_file.read_all().await?;
    let parse_one_ref = terminated(
        (
            terminated(ObjectId::parse, char(' ')),
            delimited(
                tag("refs/"),
                alt((
                    preceded(tag("heads/"), not_line_ending)
                        .map(|name: &[u8]| RefName::Branch(name.to_vec())),
                    preceded(tag("tags/"), not_line_ending)
                        .map(|name: &[u8]| RefName::Tag(name.to_vec())),
                )),
                newline,
            ),
        ),
        opt(delimited(char('^'), not_line_ending, newline)),
    )
    .map(Some);
    let parse_comment = (space0, char('#'), not_line_ending, opt(newline)).map(|_| None);
    let mut parser = many0(alt((parse_one_ref, parse_comment)));
    let (_, refs) = parser
        .parse(packed_refs_data.as_ref())
        .map_err(|_| Error::MalformedPackedRefs)?;
    Ok(refs.into_iter().flatten().collect())
}

async fn lookup_loose_ref<D: Directory>(repo: &Repo<D>, name: &RefName) -> GResult<Option<Ref>> {
    let mut ref_file = if let Some(file) = name.open_file(repo).await? {
        file
    } else {
        return Ok(None);
    };
    let ref_content = ref_file.read_all().await?;
    let (_, reference) =
        Ref::parse_loose_ref(&ref_content).map_err(|_| Error::MalformedRef(name.clone()))?;
    Ok(Some(reference))
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
        let head_target = match head {
            Ref::Direct(_) => panic!(),
            Ref::Symbolic(name) => name,
        };
        let head_target = block_on(Ref::lookup(&repo, &head_target)).unwrap();
        assert!(matches!(head_target, Ref::Direct(_)));
    }

    #[test]
    fn resolve_head_to_commit() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        let object = block_on(head.resolve_to_object(&repo)).unwrap();
        assert!(matches!(object, Object::Commit(_)));
    }

    #[test]
    fn parse_direct_ref() {
        let content = b"6121d0b97779278fcc32cc8a02754e7c588d9c18\n";
        let (_, parsed) = Ref::parse_loose_ref(content).unwrap();
        assert_eq!(
            parsed,
            Ref::Direct(ObjectId(hex!("6121d0b97779278fcc32cc8a02754e7c588d9c18"),))
        );
    }

    #[test]
    fn parse_symbolic_ref() {
        let content = b"ref: refs/heads/main\n";
        let (_, parsed) = Ref::parse_loose_ref(content).unwrap();
        assert_eq!(parsed, Ref::Symbolic(RefName::Branch(b"main".to_vec())));
    }

    #[test]
    fn read_thin_packed_ref() {
        let test_repo = make_packfile_repo().unwrap();
        let repo = test_repo.repo();
        let main = Ref::Symbolic(RefName::Branch("main".as_bytes().to_vec()));
        let object = block_on(main.resolve_to_object(&repo)).unwrap();
        assert!(matches!(object, Object::Commit(_)));
    }

    #[test]
    fn read_fat_packed_ref() {
        let test_repo = make_packfile_repo().unwrap();
        let repo = test_repo.repo();
        let main = Ref::Symbolic(RefName::Tag("a-fat-tag".as_bytes().to_vec()));
        let object = block_on(main.resolve_to_object(&repo)).unwrap();
        assert!(matches!(object, Object::Tag(_)));
    }
}
