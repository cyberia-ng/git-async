//! Traits and error types for interacting with files and directories
//!
//! The purpose of this module is to specify what consumers of this library must
//! implement in order for the filesystem calls in `git-async` to work.
//!
//! The [`FileSystem`] trait is a wrapper trait which has associated types for
//! objects which implement [`File`] and [`Directory`]. The correct way to
//! implement this is to use a zero-sized struct for the parent, and real
//! objects for [`File`] and [`Directory`]. This structure is to avoid generic
//! proliferation as much as possible.
//!
//! For example:
//!
//! ```
//! # use git_async::file_system::{File, Directory, FileSystem, FileSystemError, Offset, DirEntry};
//! struct MyFile {
//!     /* path, handle, etc. */
//! }
//!
//! impl File for MyFile {
//!     /* methods */
//!     # async fn read_all(&mut self) -> Result<Vec<u8>, FileSystemError> { unimplemented! () }
//!     # async fn read_segment(&mut self, _: Offset, _: &mut [u8]) -> Result<usize, FileSystemError> { unimplemented! () }
//! }
//!
//! struct MyDirectory {
//!     /* path, handle, etc. */
//! }
//! # impl Clone for MyDirectory { fn clone(&self) -> Self { unimplemented!() } }
//!
//! impl Directory<MyFile> for MyDirectory {
//!     /* methods */
//!     # async fn open_subdir(&self, _: &[u8]) -> Result<Self, FileSystemError> { unimplemented!() }
//!     # async fn list_dir(&self) -> Result<Vec<DirEntry>, FileSystemError> { unimplemented!() }
//!     # async fn open_file(&self, _: &[u8]) -> Result<MyFile, FileSystemError> { unimplemented!() }
//! }
//!
//! struct MyFS;
//!
//! impl FileSystem for MyFS {
//!     type Directory = MyDirectory;
//!     type File = MyFile;
//! }
//! ```
//!
//! The simplest way of implementing these would be to use [`std::fs`], but this
//! nullifies the async capabilities of this crate. If you are using Tokio for
//! example, you should use primitives from
//! [`tokio::fs`](https://docs.rs/tokio/latest/tokio/fs/). If you are using
//! `git-async` in a browser, you may want to use the [web filesystem
//! API](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API).

use alloc::{boxed::Box, vec::Vec};
use core::{any::Any, future::Future};

/// Represents a directory entry
///
/// Names are represented as [`Vec<u8>`] to work on platforms with non-Unicode
/// filename encodings.
///
/// Note that the names in this struct are **names** only, not full paths.
pub enum DirEntry {
    #[expect(missing_docs)]
    File(Vec<u8>),
    #[expect(missing_docs)]
    Directory(Vec<u8>),
}

/// A wrapper trait encapsulating filesystem objects
///
/// See the [module documentation](`self`) for further details.
pub trait FileSystem: 'static {
    /// A type for files
    type File: File;
    /// A type for a directory, which contains files
    type Directory: Directory<Self::File>;
}

/// An error encountered when doing file or directory operations.
///
/// Rather than make everything generic over the type of errors, we use
/// [`Box<dyn Any>`] to hold platform-native errors.
#[derive(Debug)]
pub enum FileSystemError {
    /// The requested file was not found
    NotFound(Box<dyn Any>),
    /// Any other kind of error
    Other(Box<dyn Any>),
}

/// A trait for directories and their operations
///
/// A simple implementation might just use a [`std::path::PathBuf`]. On
/// platforms which implement directory handles, this should encapsulate the
/// handle.
pub trait Directory<File>: Sized + Clone {
    /// Open a subdirectory of this directory
    fn open_subdir(&self, name: &[u8]) -> impl Future<Output = Result<Self, FileSystemError>>;

    /// List the entries in this directory
    fn list_dir(&self) -> impl Future<Output = Result<Vec<DirEntry>, FileSystemError>>;

    /// Open a file (for reading)
    fn open_file(&self, name: &[u8]) -> impl Future<Output = Result<File, FileSystemError>>;
}

/// An offset within a file; a newtype wrapper around a [`u64`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Offset(pub u64);

impl core::ops::Add<u64> for Offset {
    type Output = Self;
    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}
impl core::ops::Sub<u64> for Offset {
    type Output = Self;
    fn sub(self, rhs: u64) -> Self::Output {
        Self(self.0 - rhs)
    }
}
impl core::ops::Div<u64> for Offset {
    type Output = Self;

    fn div(self, rhs: u64) -> Self::Output {
        Self(self.0 / rhs)
    }
}
impl core::ops::Mul<u64> for Offset {
    type Output = Self;
    fn mul(self, rhs: u64) -> Self::Output {
        Self(self.0 * rhs)
    }
}

/// A trait for reading files
///
/// A simple (non-async) implementation would hold a [`std::fs::File`] handle.
pub trait File: Sized {
    /// Read everything in the file
    ///
    /// This should be idempotent with any other reads. If the platform requires
    /// it, implementors should seek to the start of the file before reading.
    fn read_all(&mut self) -> impl Future<Output = Result<Vec<u8>, FileSystemError>>;

    /// Read a segment of the file into the destination buffer. If less data is
    /// available than the size of the buffer, then only the first `n` bytes of
    /// the buffer are modified. Therefore implementors should take care not to
    /// error on EOF conditions.
    ///
    /// Successful reads return the number of bytes read.
    fn read_segment(
        &mut self,
        offset: Offset,
        dest: &mut [u8],
    ) -> impl Future<Output = Result<usize, FileSystemError>>;
}

pub(crate) type PathComponent = Vec<u8>;
pub(crate) type Path = Vec<PathComponent>;
enum SearchPath {
    File(Path),
    Directory(Path),
}

pub(crate) async fn search_for_files<F: File, D: Directory<F>>(
    root: &D,
) -> Result<Vec<Path>, FileSystemError> {
    use SearchPath::*;
    let mut out: Vec<Path> = Vec::new();
    let mut stack: Vec<SearchPath> = Vec::new();
    stack.push(Directory(Vec::new()));
    while let Some(this) = stack.pop() {
        match this {
            File(path) => out.push(path),
            Directory(dir) => {
                let mut dir_handle = root.clone();
                for component in &dir {
                    dir_handle = dir_handle.open_subdir(component).await?;
                }
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

#[cfg(test)]
mod tests {
    use crate::test::{directory::TestRepoDirectory, repo::TestDirectory};

    use super::*;
    use futures::executor::block_on;
    use std::{
        fs::{OpenOptions, create_dir},
        io::{self, Write},
        path::PathBuf,
        sync::Arc,
    };
    use tempfile::TempDir;

    #[test]
    fn test_search_for_files() {
        fn touch(path: impl AsRef<std::path::Path>) -> io::Result<()> {
            let mut f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(path)?;
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
            root: TestDirectory::Temp(Arc::new(dir)),
            sub_path: PathBuf::new(),
        };
        let mut paths = block_on(search_for_files(&dir)).unwrap();
        paths.sort();
        assert_eq!(paths, expected);
    }
}
