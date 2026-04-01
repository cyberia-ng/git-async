use alloc::{boxed::Box, vec::Vec};
use core::{any::Any, future::Future};

pub enum DirEntry {
    File(Vec<u8>),
    Directory(Vec<u8>),
}

#[derive(Debug)]
pub struct DirectoryError(pub Box<dyn Any>);

pub trait Directory: Sized + Clone {
    fn open_subdir(&self, name: &[u8]) -> impl Future<Output = Result<Self, DirectoryError>>;
    fn list_dir(&self) -> impl Future<Output = Result<Vec<DirEntry>, DirectoryError>>;
    fn read_file(&self, name: &[u8]) -> impl Future<Output = Result<Vec<u8>, DirectoryError>>;
}

pub(crate) type PathComponent = Vec<u8>;
pub(crate) type Path = Vec<PathComponent>;
enum SearchPath {
    File(Path),
    Directory(Path),
}

pub(crate) async fn search_for_files<D: Directory>(root: &D) -> Result<Vec<Path>, DirectoryError> {
    use SearchPath::*;
    let mut out: Vec<Path> = Vec::new();
    let mut stack: Vec<SearchPath> = Vec::new();
    stack.push(Directory(Vec::new()));
    while !stack.is_empty() {
        let this = stack.pop().unwrap();
        match this {
            File(path) => out.push(path),
            Directory(dir) => {
                let dir_handle = open_dir_path(root, &dir).await?;
                let entries = dir_handle.list_dir().await?;
                let new_stack_entries = entries.into_iter().map(|entry| {
                    let mut new_path = dir.clone();
                    match entry {
                        DirEntry::File(name) => {
                            new_path.push(name);
                            File(new_path)
                        }
                        DirEntry::Directory(name) => {
                            new_path.push(name);
                            Directory(new_path)
                        }
                    }
                });
                stack.extend(new_stack_entries);
            }
        }
    }
    Ok(out)
}

pub(crate) async fn open_dir_path<D: Directory>(
    directory: &D,
    path: &Path,
) -> Result<D, DirectoryError> {
    let mut dir = directory.clone();
    for component in path {
        dir = dir.open_subdir(&component).await?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use crate::test::repo::TestRepoDirectory;

    use super::*;
    use std::{
        fs::{OpenOptions, create_dir}, io::{self, Write}, path::PathBuf, rc::Rc
    };
    use futures::executor::block_on;
    use tempfile::TempDir;

    #[test]
    fn test_search_for_files() {
        fn touch(path: impl AsRef<std::path::Path>) -> io::Result<()> {
            let mut f = OpenOptions::new().create(true).write(true).open(path)?;
            f.flush()?;
            Ok(())
        }
        let dir = TempDir::new().unwrap();
        touch(dir.path().join("file-a")).unwrap();
        touch(dir.path().join("file-b")).unwrap();
        create_dir(dir.path().join("dir-a")).unwrap();
        touch(dir.path().join("dir-a").join("file-c")).unwrap();
        create_dir(dir.path().join("dir-a").join("dir-b")).unwrap();
        touch(dir.path().join("dir-a").join("dir-b").join("file-d")).unwrap();
        let mut expected: Vec<Path> = vec![
            vec![b"file-a".to_vec()],
            vec![b"file-b".to_vec()],
            vec![b"dir-a".to_vec(), b"file-c".to_vec()],
            vec![b"dir-a".to_vec(), b"dir-b".to_vec(), b"file-d".to_vec()],
        ];
        expected.sort();
        let dir = TestRepoDirectory {
            root: Rc::new(dir),
            sub_path: PathBuf::new(),
        };
        let mut paths = block_on(search_for_files(&dir)).unwrap();
        paths.sort();
        assert_eq!(paths, expected);
    }
}
