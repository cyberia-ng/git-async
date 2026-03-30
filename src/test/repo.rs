use crate::{
    directory::{Directory, DirectoryError},
    repo::Repo,
};
use std::{
    ffi::OsStr,
    fs::{OpenOptions, read_dir},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::{TempDir, tempdir};

#[derive(Debug)]
pub struct TestRepo {
    pub location: TempDir,
}

#[derive(Debug, Clone)]
pub struct TestRepoDirectory<'r> {
    repo: &'r TestRepo,
    sub_path: PathBuf,
}

impl TestRepo {
    pub fn run_git(
        &self,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> io::Result<Vec<u8>> {
        self.run_git_stdin(args, &[])
    }

    pub fn run_git_stdin(
        &self,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
        stdin: &[u8],
    ) -> io::Result<Vec<u8>> {
        let mut git_process = Command::new("git")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .args([OsStr::new("-C"), self.location.path().as_ref()])
            .args(args)
            .spawn()?;
        git_process.stdin.take().unwrap().write_all(stdin)?;
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
        let repo = TestRepo { location: dir };
        repo.run_git(["init"])?;
        repo.set_user("a user", "an-email-address")?;
        Ok(repo)
    }

    pub fn set_user(&self, name: &str, email: &str) -> io::Result<()> {
        self.run_git(["config", "user.name", name])?;
        self.run_git(["config", "user.email", email])?;
        Ok(())
    }

    pub fn git_dir(&self) -> TestRepoDirectory<'_> {
        TestRepoDirectory {
            repo: self,
            sub_path: PathBuf::from(".git"),
        }
    }

    pub fn working_tree_path(&self) -> &Path {
        self.location.path()
    }

    pub fn repo(&self) -> Repo<TestRepoDirectory<'_>> {
        Repo::new(self.git_dir())
    }
}

impl<'r> Directory for TestRepoDirectory<'r> {
    async fn open_subdir(&self, name: &[u8]) -> Result<Self, DirectoryError> {
        let new_sub_path = self.sub_path.join(str::from_utf8(name).unwrap());
        Ok(Self {
            repo: self.repo,
            sub_path: new_sub_path,
        })
    }

    async fn list_dir(&self) -> Result<Vec<Vec<u8>>, DirectoryError> {
        let dir = read_dir(self.repo.location.path().join(&self.sub_path)).unwrap();
        let entries = dir
            .map_while(|entry| {
                if let Ok(entry) = entry {
                    Some(entry.file_name())
                } else {
                    None
                }
            })
            .map(|file_name| file_name.to_owned().into_encoded_bytes())
            .collect::<Vec<_>>();
        Ok(entries)
    }

    async fn read_file(&self, name: &[u8]) -> Result<Vec<u8>, DirectoryError> {
        let mut file = OpenOptions::new()
            .read(true)
            .open(
                self.repo
                    .location
                    .path()
                    .join(&self.sub_path)
                    .join(str::from_utf8(name).unwrap()),
            )
            .unwrap();
        let mut out = vec![];
        file.read_to_end(&mut out).unwrap();
        Ok(out)
    }
}

pub fn make_basic_commit(test_repo: &TestRepo) {
    let wd_path = test_repo.working_tree_path();
    let mut file_path = wd_path.to_path_buf();
    file_path.push("a-file");
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file_path)
        .unwrap();
    f.flush().unwrap();
    test_repo.run_git(["add", "--all"]).unwrap();
    test_repo
        .run_git(["commit", "-m", "a commit message"])
        .unwrap();
}
