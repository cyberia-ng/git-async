use crate::{
    error::GResult,
    file_system::{Directory, FilesystemError, search_for_files},
    object::{Object, ObjectId},
    object_store::{ObjectSize, ObjectType, cache::PackFileCache},
    reference::{Ref, RefName, read_packed_refs},
    sync::SharedCell,
    traits::AllGenerics,
};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

pub(crate) struct RepoCache<G: AllGenerics> {
    pub(crate) pack_cache: Option<PackFileCache<G>>,
}

/// A handle to a Git repository
///
/// It is generic over the implementation of filesystem operations.
#[derive(Debug)]
pub struct Repo<G: AllGenerics> {
    pub(crate) git_dir: G::Directory,
    pub(crate) cache: G::SharedCell<RepoCache<G>>,
}

impl<G: AllGenerics> Clone for Repo<G> {
    fn clone(&self) -> Self {
        Self {
            git_dir: self.git_dir.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<G: AllGenerics> Repo<G> {
    /// Open the repository located at `git_dir`.
    pub fn new(git_dir: G::Directory) -> Self {
        Repo {
            git_dir,
            cache: G::SharedCell::new(RepoCache { pack_cache: None }),
        }
    }

    /// Collect all the refs tracked by the repository. Includes HEAD, branches,
    /// tags, remotes and the stash.
    pub async fn ref_names(&self) -> GResult<BTreeSet<RefName>> {
        let mut out: BTreeSet<RefName> = BTreeSet::new();
        out.insert(RefName::Head);
        match self.git_dir.open_file(b"packed-refs").await {
            Err(FilesystemError::NotFound(_)) => {}
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

    /// Get the repository's HEAD ref.
    pub async fn head(&self) -> GResult<Ref<G>> {
        Ref::lookup(self, &RefName::Head).await
    }

    /// Take a ref name and look up its content.
    pub async fn lookup_ref(&self, name: &RefName) -> GResult<Ref<G>> {
        Ref::lookup(self, name).await
    }

    /// Look up a particular object in the repository, reading the entire object
    /// into memory.
    pub async fn lookup_object(&self, id: ObjectId) -> GResult<Object<G>> {
        Object::lookup(self, id).await
    }

    /// Look up the size and type of an object, without reading it to memory or
    /// parsing its content.
    pub async fn lookup_object_size_type(&self, id: ObjectId) -> GResult<(ObjectSize, ObjectType)> {
        Object::lookup_size_type(self, id).await
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
            RefName::Ref(b"stash".to_vec()),
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

    #[test]
    fn repo_is_send() {
        fn _foo<T: Send>(_val: T) {}
        fn _bar<G>(repo: Repo<G>)
        where
            G: AllGenerics<Directory: Send, SharedCell<RepoCache<G>>: Send>,
        {
            _foo(repo);
        }
    }

    #[test]
    fn repo_is_sync() {
        fn _foo<T: Sync>(_val: T) {}
        fn _bar<G: AllGenerics<Directory: Sync, SharedCell<RepoCache<G>>: Sync>>(repo: Repo<G>) {
            _foo(repo);
        }
    }
}
