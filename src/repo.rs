use crate::{
    directory::{Directory, DirectoryError, search_for_files},
    error::GResult,
    object::{Object, ObjectId},
    reference::{Ref, RefName, read_packed_refs},
};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

pub struct Repo<D> {
    pub(crate) git_dir: D,
}

impl<D: Directory> Repo<D> {
    pub fn new(git_dir: D) -> Self {
        Repo { git_dir }
    }

    pub async fn ref_names(&self) -> GResult<BTreeSet<RefName>> {
        let mut out: BTreeSet<RefName> = BTreeSet::new();
        out.insert(RefName::Head);
        match self.git_dir.open_file(b"packed-refs").await {
            Err(DirectoryError::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
            Ok(mut packed_refs_file) => {
                let packed_refs = read_packed_refs(&mut packed_refs_file).await?;
                for (_, ref_name) in packed_refs {
                    out.insert(ref_name);
                }
            }
        }
        let refs_dir = self.git_dir.open_subdir(b"refs").await?;
        let refs_paths = search_for_files(&refs_dir).await?;
        for path in refs_paths {
            let mut name: Vec<u8> = Vec::new();
            for component in path {
                if !name.is_empty() {
                    name.push(b'/');
                }
                name.extend_from_slice(&component);
            }
            out.insert(RefName::Ref(name));
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
        test::{
            helpers::make_basic_repo,
            repo::{TestDirectory, TestRepo},
        },
    };
    use futures::executor::block_on;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn read_head() {
        let test_repo = TestRepo::new().unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        assert_eq!(
            head.ref_type(),
            &RefType::Symbolic(RefName::Ref(Vec::from(b"heads/main")))
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
        let refs = block_on(repo.ref_names()).unwrap();
        let expected: BTreeSet<_> = vec![
            RefName::Head,
            RefName::Ref(b"heads/main".to_vec()),
            RefName::Ref(b"heads/a-branch".to_vec()),
            RefName::Ref(b"heads/foo/a-branch".to_vec()),
            RefName::Ref(b"tags/thin-tag".to_vec()),
            RefName::Ref(b"tags/bar/thin-tag".to_vec()),
            RefName::Ref(b"tags/fat-tag".to_vec()),
            RefName::Ref(b"tags/a-fat-tag".to_vec()),
            RefName::Ref(b"remotes/origin/main".to_vec()),
        ]
        .into_iter()
        .collect();
        assert_eq!(&refs, &expected);
    }
}
