use crate::{
    directory::{Directory, DirectoryError, search_for_files},
    error::GResult,
    object::{Object, ObjectId},
    reference::{Ref, RefName, read_packed_refs},
};
use alloc::vec::Vec;

pub struct Repo<D> {
    pub(crate) git_dir: D,
}

impl<D: Directory> Repo<D> {
    pub fn new(git_dir: D) -> Self {
        Repo { git_dir }
    }

    pub async fn ref_names(&self) -> GResult<Vec<RefName>> {
        let mut out: Vec<RefName> = Vec::new();
        out.push(RefName::Head);
        match self.git_dir.open_file(b"packed-refs").await {
            Err(DirectoryError::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
            Ok(mut packed_refs_file) => {
                let packed_refs = read_packed_refs(&mut packed_refs_file).await?;
                out.extend(packed_refs.into_iter().map(|(_id, name)| name));
            }
        }
        let refs_dir = self.git_dir.open_subdir(b"refs").await?;
        let refs_paths = search_for_files(&refs_dir).await?;
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

    pub async fn head(&self) -> GResult<Ref<'_, D>> {
        Ref::lookup(self, &RefName::Head).await
    }

    pub async fn lookup_ref(&self, name: &RefName) -> GResult<Ref<'_, D>> {
        Ref::lookup(self, name).await
    }

    pub async fn lookup_object(&self, id: ObjectId) -> GResult<Object<'_, D>> {
        Object::lookup(self, id).await
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        reference::RefType,
        test::{helpers::make_basic_repo, repo::TestRepo},
    };
    use futures::executor::block_on;

    use super::*;

    #[test]
    fn read_head() {
        let test_repo = TestRepo::new().unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        assert_eq!(
            head.ref_type(),
            &RefType::Symbolic(RefName::Branch(Vec::from(b"main")))
        );
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
        let mut refs = block_on(repo.ref_names()).unwrap();
        refs.sort();
        let mut expected = vec![
            RefName::Head,
            RefName::Branch(b"main".to_vec()),
            RefName::Branch(b"a-branch".to_vec()),
            RefName::Branch(b"foo/a-branch".to_vec()),
            RefName::Tag(b"thin-tag".to_vec()),
            RefName::Tag(b"bar/thin-tag".to_vec()),
            RefName::Tag(b"fat-tag".to_vec()),
            RefName::Tag(b"a-fat-tag".to_vec()),
            RefName::Remote(b"origin/main".to_vec()),
        ];
        expected.sort();
        assert_eq!(&refs, &expected);
    }
}
