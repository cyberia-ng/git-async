use crate::{
    directory::Directory,
    error::{Error, GResult},
    reference::{Ref, RefName},
};
use alloc::vec::Vec;

#[derive(Debug)]
pub struct Repo<D> {
    pub(crate) git_dir: D,
}

impl<D: Directory> Repo<D> {
    pub fn new(git_dir: D) -> Self {
        Repo { git_dir }
    }

    pub async fn head(&self) -> GResult<Ref> {
        let ref_content = self.git_dir.read_file(b"HEAD").await?;
        let (_, reference) =
            Ref::parse(&ref_content).map_err(|_| Error::MalformedRef(RefName::Head))?;
        Ok(reference)
    }

    pub async fn branches(&self) -> GResult<Vec<RefName>> {
        let refs_dir = self.git_dir.open_subdir(b"refs").await?;
        let heads_dir = refs_dir.open_subdir(b"heads").await?;
        let branch_names = heads_dir.list_dir().await?;
        Ok(branch_names.into_iter().map(RefName::Branch).collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        reference::{Ref, RefName},
        test::repo::{TestRepo, make_basic_commit},
    };
    use futures::executor::block_on;

    #[test]
    fn read_head() {
        let test_repo = TestRepo::new().unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        assert_eq!(head, Ref::Symbolic(RefName::Branch(Vec::from(b"main"))));
    }

    #[test]
    fn read_branches() {
        let test_repo = TestRepo::new().unwrap();
        make_basic_commit(&test_repo);
        test_repo.run_git(["branch", "a-branch"]).unwrap();
        let repo = test_repo.repo();
        let mut branches = block_on(repo.branches()).unwrap();
        branches.sort();
        assert_eq!(
            &branches,
            &[
                RefName::Branch(Vec::from(b"a-branch")),
                RefName::Branch(Vec::from(b"main"))
            ]
        );
    }
}
