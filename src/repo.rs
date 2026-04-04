use crate::{
    directory::{Directory, File, search_for_files},
    error::{Error, GResult},
    reference::{Ref, RefName},
};
use alloc::vec::Vec;

pub struct Repo<D> {
    pub(crate) git_dir: D,
}

impl<D: Directory> Repo<D> {
    pub fn new(git_dir: D) -> Self {
        Repo { git_dir }
    }

    pub async fn refs(&self) -> GResult<Vec<RefName>> {
        let refs_dir = self.git_dir.open_subdir(b"refs").await?;
        let refs_paths = search_for_files(&refs_dir).await?;
        let mut out: Vec<RefName> = Vec::new();
        out.push(RefName::Head);
        for path in refs_paths {
            let (prefix, rest) = path.split_at(1);
            if let Some(prefix) = prefix.first() {
                let mut name: Vec<u8> = Vec::new();
                for component in rest {
                    if !name.is_empty() {
                        name.push(b'/');
                    }
                    name.extend_from_slice(component);
                }
                match prefix.as_slice() {
                    b"heads" => {
                        out.push(RefName::Branch(name));
                    }
                    b"tags" => {
                        out.push(RefName::Tag(name));
                    }
                    b"remotes" => {
                        out.push(RefName::Remote(name));
                    }
                    _ => {}
                }
            }
        }
        Ok(out)
    }

    pub async fn head(&self) -> GResult<Ref> {
        let ref_content = self.git_dir.open_file(b"HEAD").await?.read_all().await?;
        let (_, reference) =
            Ref::parse(&ref_content).map_err(|_| Error::MalformedRef(RefName::Head))?;
        Ok(reference)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        reference::{Ref, RefName},
        test::{helpers::make_basic_repo, repo::TestRepo},
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
    fn read_refs() {
        let test_repo = make_basic_repo().unwrap();
        test_repo.run_git(["branch", "a-branch"]).unwrap();
        test_repo.run_git(["branch", "foo/a-branch"]).unwrap();
        test_repo.run_git(["tag", "thin-tag"]).unwrap();
        test_repo.run_git(["tag", "bar/thin-tag"]).unwrap();
        test_repo
            .run_git(["tag", "-a", "-m", "a tag message", "fat-tag"])
            .unwrap();
        test_repo
            .run_git(["update-ref", "refs/remotes/origin/main", "HEAD"])
            .unwrap();

        let repo = test_repo.repo();
        let mut refs = block_on(repo.refs()).unwrap();
        refs.sort();
        let mut expected = vec![
            RefName::Head,
            RefName::Branch(b"main".to_vec()),
            RefName::Branch(b"a-branch".to_vec()),
            RefName::Branch(b"foo/a-branch".to_vec()),
            RefName::Tag(b"thin-tag".to_vec()),
            RefName::Tag(b"bar/thin-tag".to_vec()),
            RefName::Tag(b"fat-tag".to_vec()),
            RefName::Remote(b"origin/main".to_vec()),
        ];
        expected.sort();
        assert_eq!(&refs, &expected);
    }
}
