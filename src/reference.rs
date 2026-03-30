use crate::{
    directory::Directory,
    error::{Error, GResult},
    object::{Object, ObjectId},
    repo::Repo,
};
use alloc::vec::Vec;
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take_till},
    character::complete::{newline, not_line_ending},
    combinator::all_consuming,
    sequence::{preceded, terminated},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum RefName {
    Branch(Vec<u8>),
    Tag(Vec<u8>),
    Remote(Vec<u8>),
    Head,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Ref {
    Direct(ObjectId),
    Symbolic(RefName),
}

impl Ref {
    pub async fn lookup<D: Directory>(repo: &Repo<D>, name: &RefName) -> GResult<Ref> {
        match name {
            RefName::Head => {
                let ref_content = repo.git_dir.read_file(b"HEAD").await?;
                let (_, reference) =
                    Ref::parse(&ref_content).map_err(|_| Error::MalformedRef(name.clone()))?;
                Ok(reference)
            }
            RefName::Branch(branch_name) => {
                let mut path_components = branch_name.split(|b| *b == b'/');
                let file_name = path_components
                    .next_back()
                    .ok_or(Error::PathError(branch_name.clone()))?;
                let mut dir = repo.git_dir.open_subdir(b"refs").await?;
                dir = dir.open_subdir(b"heads").await?;
                for component in path_components {
                    dir = dir.open_subdir(component).await?;
                }
                let file_content = dir.read_file(file_name).await?;
                let (_, reference) =
                    Self::parse(&file_content).map_err(|_| Error::MalformedRef(name.clone()))?;
                Ok(reference)
            }
            RefName::Tag(_) => todo!(),
            RefName::Remote(_) => todo!(),
        }
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

    pub(crate) fn parse(content: &[u8]) -> nom::IResult<&[u8], Self> {
        all_consuming(terminated(not_line_ending, newline))
            .and_then(alt((
                ObjectId::parse.map(Ref::Direct),
                preceded(
                    tag("ref: refs/"),
                    alt((
                        preceded(tag("heads/"), take_till(|_| false))
                            .map(|name: &[u8]| Ref::Symbolic(RefName::Branch(name.to_vec()))),
                        preceded(tag("tags/"), take_till(|_| false)).map(|_| todo!()),
                        preceded(tag("remotes/"), take_till(|_| false)).map(|_| todo!()),
                    )),
                ),
            )))
            .parse(content)
    }
}

#[cfg(test)]
mod test {
    use crate::{
        object::{Object, ObjectId},
        reference::{Ref, RefName},
        test::repo::{TestRepo, make_basic_commit},
    };
    use futures::executor::block_on;
    use hex_literal::hex;

    #[test]
    fn resolve_head() {
        let test_repo = TestRepo::new().unwrap();
        make_basic_commit(&test_repo);

        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        let head_target = match head {
            Ref::Direct(_) => panic!(),
            Ref::Symbolic(name) => name,
        };
        let head_target = block_on(Ref::lookup(&repo, &head_target)).unwrap();
        match head_target {
            Ref::Symbolic(_) => panic!(),
            Ref::Direct(_) => {
                // Happy
            }
        }
    }

    #[test]
    fn resolve_head_to_commit() {
        let test_repo = TestRepo::new().unwrap();
        make_basic_commit(&test_repo);

        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        let object = block_on(head.resolve_to_object(&repo)).unwrap();
        match object {
            Object::Commit(_) => {
                // Happy
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_direct_ref() {
        let content = b"6121d0b97779278fcc32cc8a02754e7c588d9c18\n";
        let (_, parsed) = Ref::parse(content).unwrap();
        assert_eq!(
            parsed,
            Ref::Direct(ObjectId(hex!("6121d0b97779278fcc32cc8a02754e7c588d9c18"),))
        );
    }

    #[test]
    fn parse_symbolic_ref() {
        let content = b"ref: refs/heads/main\n";
        let (_, parsed) = Ref::parse(content).unwrap();
        assert_eq!(parsed, Ref::Symbolic(RefName::Branch(b"main".to_vec())));
    }
}
