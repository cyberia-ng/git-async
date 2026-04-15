use crate::{
    file_system::{DirEntry, Directory, File, FilesystemError, Offset},
    repo::Repo,
};
use core::cmp::min;
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions, metadata, read_dir},
    io::{self, Read, Seek, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    rc::Rc,
};
use tempfile::{TempDir, tempdir};

#[derive(Debug, Clone)]
pub enum TestDirectory {
    Temp(Rc<TempDir>),

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

#[derive(Debug, Clone)]
pub struct TestRepoDirectory {
    pub root: TestDirectory,
    pub sub_path: PathBuf,
}

#[derive(Debug)]
pub struct TestRepoFile {
    pub file: fs::File,
    _dir: TestDirectory,
}

impl TestRepo {
    pub fn run_git(
        &self,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> io::Result<Vec<u8>> {
        let mut git_process = Command::new("git")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(self.location.path())
            .args(args)
            .spawn()?;
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
            location: TestDirectory::Temp(Rc::new(dir)),
        };
        repo.run_git(["init"])?;
        repo.set_user("a user", "an-email-address")?;
        Ok(repo)
    }

    fn set_user(&self, name: &str, email: &str) -> io::Result<()> {
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

    pub fn repo(&self) -> Repo<TestRepoDirectory> {
        Repo::new(self.git_dir())
    }

    pub fn commit(
        &self,
        message: &str,
        author_name: &str,
        author_email: &str,
        date: &str,
    ) -> io::Result<()> {
        self.set_user(author_name, author_email)?;
        let mut p = Command::new("git")
            .current_dir(self.location.path())
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
        let mut p = Command::new("git")
            .current_dir(self.location.path())
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

impl Directory for TestRepoDirectory {
    type File = TestRepoFile;

    async fn open_subdir(&self, name: &[u8]) -> Result<Self, FilesystemError> {
        let new_sub_path = self.sub_path.join(str::from_utf8(name).unwrap());
        if let Err(e) = metadata(self.root.path().join(&new_sub_path))
            && e.kind() == io::ErrorKind::NotFound
        {
            return Err(FilesystemError::NotFound(Box::new(e)));
        }
        Ok(Self {
            root: self.root.clone(),
            sub_path: new_sub_path,
        })
    }

    async fn list_dir(&self) -> Result<Vec<DirEntry>, FilesystemError> {
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

    async fn open_file(&self, name: &[u8]) -> Result<Self::File, FilesystemError> {
        let file = OpenOptions::new().read(true).open(
            self.root
                .path()
                .join(&self.sub_path)
                .join(str::from_utf8(name).unwrap()),
        );
        let file = match file {
            Ok(f) => f,
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    return Err(FilesystemError::NotFound(Box::new(e)));
                } else {
                    return Err(FilesystemError::Other(Box::new(e)));
                }
            }
        };
        Ok(TestRepoFile {
            file,
            _dir: self.root.clone(),
        })
    }
}

impl File for TestRepoFile {
    async fn read_all(&mut self) -> Result<Vec<u8>, FilesystemError> {
        self.file.seek(io::SeekFrom::Start(0)).unwrap();
        let mut out = vec![];
        self.file.read_to_end(&mut out).unwrap();
        Ok(out)
    }

    async fn read_segment(
        &mut self,
        offset: Offset,
        dest: &mut [u8],
    ) -> Result<usize, FilesystemError> {
        let metadata = self.file.metadata().unwrap();
        let available_len = metadata.len() - offset.0;
        let read_len = min(usize::try_from(available_len).unwrap(), dest.len());
        self.file.seek(io::SeekFrom::Start(offset.0)).unwrap();
        self.file.read_exact(&mut dest[0..read_len]).unwrap();
        Ok(read_len)
    }
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
        for (idx, item) in test_contents.iter_mut().enumerate() {
            *item = (idx % 256).try_into().unwrap();
        }
        let dir = tempdir().unwrap();
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.path().join("a-file"))
            .unwrap();
        f.write_all(&test_contents).unwrap();
        let dir = TestRepoDirectory {
            root: TestDirectory::Temp(Rc::new(dir)),
            sub_path: PathBuf::new(),
        };
        let offset = Offset(700);
        let length: usize = 32;
        let mut file = block_on(dir.open_file(b"a-file")).unwrap();
        let mut content = vec![0u8; length];
        block_on(file.read_segment(offset, &mut content)).unwrap();
        assert_eq!(content.len(), length);
        assert_eq!(
            &content,
            &test_contents[(offset.0 as usize)..((offset.0 as usize) + length)]
        );
    }
}
