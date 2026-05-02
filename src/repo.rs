use crate::{
    error::{Error, GResult},
    file_system::{Directory, FileSystem, FileSystemError, search_for_files},
    object::{Object, ObjectId},
    object_store::{ObjectSize, ObjectType, cache::IndexCache},
    reference::{Ref, RefName, read_packed_refs},
};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

/// Configuration for opening a repository
pub struct RepoConfig {
    pub(crate) index_offset_cache_max: usize,
}
impl RepoConfig {
    /// Construct a default [`RepoConfig`].
    ///
    /// See [`RepoConfig::default()`] for further details.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum size of the cache that holds object offsets from pack index files.
    pub fn index_offset_cache_max(&mut self, size: usize) -> &mut Self {
        self.index_offset_cache_max = size;
        self
    }

    /// Open a repo with this configuration.
    pub async fn open<F: FileSystem>(&self, open_dir: F::Directory) -> GResult<Repo<F>> {
        Repo::open_with_config(open_dir, self).await
    }
}

impl Default for RepoConfig {
    /// Creates a default [`RepoConfig`].
    ///
    /// The default maximum size of the index offset cache is 64 MiB.
    fn default() -> Self {
        Self {
            index_offset_cache_max: 64 * 1024 * 1024,
        }
    }
}

/// A handle to a Git repository
///
/// It is generic over the implementation of filesystem operations.
pub struct Repo<F: FileSystem> {
    pub(crate) git_dir: F::Directory,
    pub(crate) pack_dir: F::Directory,
    pub(crate) index_cache: IndexCache,
}

impl<F: FileSystem> Repo<F> {
    pub(crate) async fn open_with_config(
        open_dir: F::Directory,
        config: &RepoConfig,
    ) -> GResult<Self> {
        let git_dir = Self::resolve_git_dir(open_dir).await?;
        let pack_dir = git_dir
            .open_subdir(b"objects")
            .await?
            .open_subdir(b"pack")
            .await?;
        let index_cache = IndexCache::new(&pack_dir, config).await?;
        Ok(Repo {
            git_dir,
            pack_dir,
            index_cache,
        })
    }

    pub(crate) async fn resolve_git_dir(open_dir: F::Directory) -> GResult<F::Directory> {
        let head = open_dir.open_file(b"HEAD").await;
        match head {
            Ok(_) => Ok(open_dir),
            Err(FileSystemError::NotFound(_)) => {
                let git_dir = open_dir.open_subdir(b".git").await?;
                let head = git_dir.open_file(b"HEAD").await;
                match head {
                    Ok(_) => Ok(git_dir),
                    Err(FileSystemError::NotFound(_)) => Err(Error::NotAGitRepository),
                    Err(e) => Err(e.into()),
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Open the repository located at `git_dir` using a default [`RepoConfig`].
    pub async fn open(git_dir: F::Directory) -> GResult<Self> {
        Self::open_with_config(git_dir, &RepoConfig::default()).await
    }

    /// Collect all the refs tracked by the repository
    ///
    /// Includes HEAD, branches, tags, remotes and the stash
    pub async fn ref_names(&self) -> GResult<BTreeSet<RefName>> {
        let mut out: BTreeSet<RefName> = BTreeSet::new();
        out.insert(RefName::Head);
        match self.git_dir.open_file(b"packed-refs").await {
            Err(FileSystemError::NotFound(_)) => {}
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
    pub async fn head(&self) -> GResult<Ref> {
        Ref::lookup(self, &RefName::Head).await
    }

    /// Take a ref name and look up its content.
    pub async fn lookup_ref(&self, name: &RefName) -> GResult<Ref> {
        Ref::lookup(self, name).await
    }

    /// Look up a particular object in the repository, reading the entire object
    /// into memory.
    pub async fn lookup_object(&self, id: ObjectId) -> GResult<Object> {
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
    use super::*;
    use crate::{
        reference::RefTarget,
        test::{helpers::make_basic_repo, impls::TestFileSystem, repo::TestRepo},
    };
    use futures::executor::block_on;

    #[test]
    fn read_head() {
        let test_repo = TestRepo::new().unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        assert_eq!(
            head.target(),
            &RefTarget::Symbolic(RefName::Ref(Vec::from(b"heads/main")))
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
    fn open_non_bare_repo() {
        let test_repo = make_basic_repo().unwrap();
        let root_dir = test_repo.root_dir();
        block_on(Repo::<TestFileSystem>::open(root_dir)).unwrap();
    }
}
