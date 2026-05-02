use crate::{
    repo::{Repo, RepoConfig},
    test::{
        directory::{TestRepoDirectory, TestRepoFile},
        impls::TestFileSystem,
    },
};

use futures::executor::block_on;
use std::{
    ffi::OsStr,
    fs::OpenOptions,
    io::{self, Read},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};
use tempfile::{TempDir, tempdir};

#[derive(Debug, Clone)]
pub enum TestDirectory {
    Temp(Arc<TempDir>),

    // This is for debugging operations on real repos, the tests for which are
    // not to be committed.
    #[allow(dead_code)]
    Real(PathBuf),
}

impl TestDirectory {
    pub fn path(&self) -> &Path {
        use TestDirectory::*;
        match self {
            Temp(d) => d.path(),
            Real(d) => d.as_path(),
        }
    }

    #[allow(dead_code)]
    /// Keep the test directory around for debugging
    pub fn forget(&self) {
        use TestDirectory::*;
        match self {
            Temp(d) => {
                std::mem::forget(d.clone());
                println!("{:?}", d.path());
            }
            Real(_) => {}
        }
    }
}

#[derive(Debug)]
pub struct TestRepo {
    pub location: TestDirectory,
}

impl TestRepo {
    pub fn git_command(&self) -> Command {
        let mut command = Command::new("git");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(self.location.path())
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_AUTHOR_DATE")
            .env_remove("GIT_COMMITTER_NAME")
            .env_remove("GIT_COMMITTER_EMAIL")
            .env_remove("GIT_COMMITTER_DATE");
        command
    }
    pub fn run_git(
        &self,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> io::Result<Vec<u8>> {
        let mut git_process = self.git_command().args(args).spawn()?;
        let status = git_process.wait()?;
        assert!(status.success());
        let mut output = Vec::new();
        git_process
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut output)?;
        Ok(output)
    }

    pub fn new() -> io::Result<Self> {
        let dir = tempdir()?;
        let repo = TestRepo {
            location: TestDirectory::Temp(Arc::new(dir)),
        };
        repo.run_git(["init", "--initial-branch=main"])?;
        repo.set_user("a user", "an-email-address")?;
        Ok(repo)
    }

    fn set_user(&self, name: &str, email: &str) -> io::Result<()> {
        self.run_git(["config", "user.name", name])?;
        self.run_git(["config", "user.email", email])?;
        Ok(())
    }

    pub fn root_dir(&self) -> TestRepoDirectory {
        TestRepoDirectory {
            root: self.location.clone(),
            sub_path: PathBuf::new(),
        }
    }

    pub fn git_dir(&self) -> TestRepoDirectory {
        TestRepoDirectory {
            root: self.location.clone(),
            sub_path: PathBuf::from(".git"),
        }
    }

    pub fn repo(&self) -> Repo<TestFileSystem> {
        block_on(RepoConfig::default().open(self.git_dir())).unwrap()
    }

    pub fn commit(
        &self,
        message: &str,
        author_name: &str,
        author_email: &str,
        date: &str,
    ) -> io::Result<()> {
        self.set_user(author_name, author_email)?;
        let mut p = self
            .git_command()
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .args(["commit", "-m", message])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let status = p.wait().unwrap();
        assert!(status.success());
        Ok(())
    }

    pub fn tag_annotated(
        &self,
        tag_name: &str,
        object: &str,
        message: &str,
        author_name: &str,
        author_email: &str,
        date: &str,
    ) -> io::Result<()> {
        self.set_user(author_name, author_email)?;
        let mut p = self
            .git_command()
            .env("GIT_COMMITTER_DATE", date)
            .args(["tag", "-a", "-m", message, tag_name, object])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let status = p.wait().unwrap();
        assert!(status.success());
        Ok(())
    }

    fn pack_dir_path(&self) -> PathBuf {
        self.location
            .path()
            .join(".git")
            .join("objects")
            .join("pack")
            .clone()
    }

    pub fn pack_idx_file(&self, pack_id: &[u8]) -> io::Result<TestRepoFile> {
        let mut idx_name = Vec::new();
        idx_name.extend_from_slice(b"pack-");
        idx_name.extend_from_slice(pack_id);
        idx_name.extend_from_slice(b".idx");
        let file = OpenOptions::new()
            .read(true)
            .open(self.pack_dir_path().join(OsStr::from_bytes(&idx_name)))?;
        Ok(TestRepoFile {
            file,
            _dir: self.location.clone(),
        })
    }
}
