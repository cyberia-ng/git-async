use crate::{
    directory::{DirEntry, Directory, DirectoryError, File},
    repo::Repo,
};
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions, read_dir},
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    rc::Rc,
};
use tempfile::{TempDir, tempdir};

#[derive(Debug)]
pub struct TestRepo {
    pub location: Rc<TempDir>,
}

#[derive(Debug, Clone)]
pub struct TestRepoDirectory {
    pub root: Rc<TempDir>,
    pub sub_path: PathBuf,
}

#[derive(Debug)]
pub struct TestRepoFile {
    pub file: fs::File,
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
        let repo = TestRepo {
            location: Rc::new(dir),
        };
        repo.run_git(["init"])?;
        repo.set_user("a user", "an-email-address")?;
        Ok(repo)
    }

    pub fn set_user(&self, name: &str, email: &str) -> io::Result<()> {
        self.run_git(["config", "user.name", name])?;
        self.run_git(["config", "user.email", email])?;
        Ok(())
    }

    pub fn git_dir(&self) -> TestRepoDirectory {
        TestRepoDirectory {
            root: self.location.clone(),
            sub_path: PathBuf::from(".git"),
        }
    }

    pub fn working_tree_path(&self) -> &Path {
        self.location.path()
    }

    pub fn repo(&self) -> Repo<TestRepoDirectory> {
        Repo::new(self.git_dir())
    }
}

impl Directory for TestRepoDirectory {
    type File = TestRepoFile;

    async fn open_subdir(&self, name: &[u8]) -> Result<Self, DirectoryError> {
        let new_sub_path = self.sub_path.join(str::from_utf8(name).unwrap());
        Ok(Self {
            root: self.root.clone(),
            sub_path: new_sub_path,
        })
    }

    async fn list_dir(&self) -> Result<Vec<DirEntry>, DirectoryError> {
        let dir = read_dir(self.root.path().join(&self.sub_path)).unwrap();
        let entries = dir
            .map_while(|entry| {
                if let Ok(entry) = entry {
                    let file_type = entry.file_type().unwrap();
                    let file_name = entry.file_name().into_encoded_bytes();
                    if file_type.is_dir() {
                        Some(DirEntry::Directory(file_name))
                    } else if file_type.is_file() {
                        Some(DirEntry::File(file_name))
                    } else {
                        panic!("symlinks not supported in tests");
                    }
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        Ok(entries)
    }

    async fn open_file(&self, name: &[u8]) -> Result<Self::File, DirectoryError> {
        let file = OpenOptions::new()
            .read(true)
            .open(
                self.root
                    .path()
                    .join(&self.sub_path)
                    .join(str::from_utf8(name).unwrap()),
            )
            .unwrap();
        Ok(TestRepoFile { file })
    }
}

impl File for TestRepoFile {
    async fn read_all(&mut self) -> Result<Vec<u8>, DirectoryError> {
        self.file.seek(io::SeekFrom::Start(0)).unwrap();
        let mut out = vec![];
        self.file.read_to_end(&mut out).unwrap();
        Ok(out)
    }

    async fn read_segment(&mut self, offset: u64, dest: &mut [u8]) -> Result<(), DirectoryError> {
        self.file.seek(io::SeekFrom::Start(offset)).unwrap();
        self.file.read_exact(dest).unwrap();
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use std::fs::OpenOptions;
    use tempfile::tempdir;

    #[test]
    fn test_seek_offset() {
        let mut test_contents = vec![0u8; 1024];
        for idx in 0..test_contents.len() {
            test_contents[idx] = (idx % 256).try_into().unwrap();
        }
        let dir = tempdir().unwrap();
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.path().join("a-file"))
            .unwrap();
        f.write_all(&test_contents).unwrap();
        let dir = TestRepoDirectory {
            root: Rc::new(dir),
            sub_path: PathBuf::new(),
        };
        let offset: usize = 700;
        let length: usize = 32;
        let mut file = block_on(dir.open_file(b"a-file")).unwrap();
        let mut content = vec![0u8; length];
        block_on(file.read_segment(offset.try_into().unwrap(), &mut content)).unwrap();
        assert_eq!(content.len(), length);
        assert_eq!(&content, &test_contents[offset..(offset + length)]);
    }
}
