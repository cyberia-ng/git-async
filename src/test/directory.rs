use crate::{
    file_system::{DirEntry, Directory, File, FileSystemError, Offset},
    test::repo::TestDirectory,
};
use core::cmp::min;
use std::{
    fs,
    io::{self, Read, Seek, Write},
    path::PathBuf,
};

#[derive(Debug, Clone)]
pub struct TestRepoDirectory {
    pub root: TestDirectory,
    pub sub_path: PathBuf,
}

#[derive(Debug)]
pub struct TestRepoFile {
    pub file: fs::File,
    pub _dir: TestDirectory,
}

impl Directory<TestRepoFile> for TestRepoDirectory {
    async fn open_subdir(&self, name: &[u8]) -> Result<Self, FileSystemError> {
        let new_sub_path = self.sub_path.join(str::from_utf8(name).unwrap());
        if let Err(e) = fs::metadata(self.root.path().join(&new_sub_path))
            && e.kind() == io::ErrorKind::NotFound
        {
            return Err(FileSystemError::NotFound(Box::new(e)));
        }
        Ok(Self {
            root: self.root.clone(),
            sub_path: new_sub_path,
        })
    }

    async fn list_dir(&self) -> Result<Vec<DirEntry>, FileSystemError> {
        let dir = fs::read_dir(self.root.path().join(&self.sub_path)).unwrap();
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

    async fn open_file(&self, name: &[u8]) -> Result<TestRepoFile, FileSystemError> {
        let file = fs::OpenOptions::new().read(true).open(
            self.root
                .path()
                .join(&self.sub_path)
                .join(str::from_utf8(name).unwrap()),
        );
        let file = match file {
            Ok(f) => f,
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    return Err(FileSystemError::NotFound(Box::new(e)));
                }
                return Err(FileSystemError::Other(Box::new(e)));
            }
        };
        Ok(TestRepoFile {
            file,
            _dir: self.root.clone(),
        })
    }
}

impl File for TestRepoFile {
    async fn read_all(&mut self) -> Result<Vec<u8>, FileSystemError> {
        self.file.seek(io::SeekFrom::Start(0)).unwrap();
        let mut out = vec![];
        self.file.read_to_end(&mut out).unwrap();
        Ok(out)
    }

    async fn read_segment(
        &mut self,
        offset: Offset,
        dest: &mut [u8],
    ) -> Result<usize, FileSystemError> {
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
    use std::{fs::OpenOptions, sync::Arc};
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
            root: TestDirectory::Temp(Arc::new(dir)),
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
