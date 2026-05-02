//! An implementation of filesystem operations for the web
//!
//! This module implements the `git-async` filesystem operations using either
//! the [Web File System
//! API](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API) or
//! the [File and Directory Entries
//! API](https://developer.mozilla.org/en-US/docs/Web/API/File_and_Directory_Entries_API).
//!
//! # Examples
//!
//! Using the Web File System API:
//! ```
//! # use wasm_bindgen::JsError;
//! # use git_async::{Repo, RepoConfig, web::{WebDirectory, WebFileSystem}};
//! async fn open_repo(handle: &web_sys::FileSystemDirectoryHandle) -> Result<Repo<WebFileSystem>, JsError> {
//!     let repo = Repo::open(WebDirectory::new(handle)?)
//!         .await
//!         .map_err(|e| JsError::new(&format!("{:?}", e)))?;
//!     Ok(repo)
//! }
//! ```
//!
//! Using the Web File and Directory Entries API:
//! ```
//! # use wasm_bindgen::JsError;
//! # use git_async::{Repo, RepoConfig, web::{WebDirectory, WebFileSystem}};
//! async fn open_repo(file_list: &web_sys::FileList) -> Result<Repo<WebFileSystem>, JsError> {
//!     let repo = Repo::open(WebDirectory::new(file_list)?)
//!         .await
//!         .map_err(|e| JsError::new(&format!("{:?}", e)))?;
//!     Ok(repo)
//! }
//! ```

use crate::file_system::{DirEntry, Directory, File, FileSystem, FileSystemError, Offset};
use alloc::{boxed::Box, string::String, vec, vec::Vec};
use js_sys::{Array, JsString, Promise, Reflect, TypeError, Uint8Array};
use wasm_bindgen::prelude::*;
use web_sys::{DomException, FileList, FileSystemDirectoryHandle};

fn to_filesystem_error(value: JsValue) -> FileSystemError {
    if value.has_type::<DomException>()
        && Reflect::get(&value, &JsValue::from("name")).unwrap() == "NotFoundError"
    {
        FileSystemError::NotFound(Box::new(value))
    } else if let Ok(s) = value.clone().dyn_into::<JsString>()
        && s == "file not found"
    {
        FileSystemError::NotFound(Box::new(value))
    } else {
        FileSystemError::Other(Box::new(value))
    }
}

#[wasm_bindgen(module = "/src/web/file-system.js")]
extern "C" {
    type DirectoryWrapper;
    #[wasm_bindgen(constructor)]
    fn new(inner: &JsValue) -> DirectoryWrapper;

    #[wasm_bindgen(method)]
    fn openSubdir(this: &DirectoryWrapper, name: &str) -> Promise;
    #[wasm_bindgen(method)]
    fn listDir(this: &DirectoryWrapper) -> Promise;
    #[wasm_bindgen(method)]
    fn openFile(this: &DirectoryWrapper, name: &str) -> Promise;

    type FileWrapper;
    #[wasm_bindgen(method)]
    fn readAll(this: &FileWrapper) -> Promise;
    #[wasm_bindgen(method)]
    fn readSegment(this: &FileWrapper, offset: f64, length: f64) -> Promise;

    type FSDirectory;
    #[wasm_bindgen(constructor)]
    fn new(handle: &FileSystemDirectoryHandle) -> FSDirectory;

    type EntriesDirectory;
    fn entriesDirectoryFromFileList(fileList: &FileList) -> EntriesDirectory;
}

/// File system operations using web APIs
pub struct WebFileSystem;
impl FileSystem for WebFileSystem {
    type File = WebFile;
    type Directory = WebDirectory;
}

/// A provider for the [`Directory`] trait which delegates to web APIs
pub struct WebDirectory {
    directory: DirectoryWrapper,
}

impl Clone for WebDirectory {
    fn clone(&self) -> Self {
        Self {
            directory: self.directory.clone().dyn_into().unwrap(),
        }
    }
}

impl WebDirectory {
    /// Construct a new [`WebDirectory`] from either a
    /// [`web_sys::FileSystemDirectoryHandle`] or a [`web_sys::FileList`].
    ///
    /// If the argument is not one of these, this function returns an `Err`.
    pub fn new(inner: &JsValue) -> Result<Self, JsError> {
        if let Ok(handle) = inner.clone().dyn_into::<FileSystemDirectoryHandle>() {
            let directory = FSDirectory::new(&handle);
            Ok(Self {
                directory: DirectoryWrapper::new(&directory),
            })
        } else if let Ok(file_list) = inner.clone().dyn_into::<web_sys::FileList>() {
            let directory = entriesDirectoryFromFileList(&file_list);
            Ok(Self {
                directory: DirectoryWrapper::new(&directory),
            })
        } else {
            Err(JsError::new(
                "must provide either a FileSystemDirectory Handle or a FileList object",
            ))
        }
    }
}

impl Directory<WebFile> for WebDirectory {
    async fn open_subdir(&self, name: &[u8]) -> Result<Self, FileSystemError> {
        let f = async || -> Result<Self, JsValue> {
            let subdir: DirectoryWrapper = self
                .directory
                .openSubdir(str::from_utf8(name).map_err(|_| TypeError::new("name was not UTF-8"))?)
                .await?
                .dyn_into()?;
            Ok(Self { directory: subdir })
        };
        f().await.map_err(to_filesystem_error)
    }

    async fn list_dir(&self) -> Result<Vec<DirEntry>, FileSystemError> {
        let f = async || -> Result<Vec<DirEntry>, JsValue> {
            let entries: Array = self.directory.listDir().await?.dyn_into()?;
            let directories: Array = entries.at(0).dyn_into()?;
            let files: Array = entries.at(1).dyn_into()?;
            let mut out: Vec<DirEntry> = Vec::new();
            for name in directories {
                let name: JsString = name.dyn_into()?;
                let name: String = name.into();
                let name: Vec<u8> = name.into_bytes();
                out.push(DirEntry::Directory(name));
            }
            for name in files {
                let name: JsString = name.dyn_into()?;
                let name: String = name.into();
                let name: Vec<u8> = name.into_bytes();
                out.push(DirEntry::File(name));
            }
            Ok(out)
        };
        f().await.map_err(to_filesystem_error)
    }

    async fn open_file(&self, name: &[u8]) -> Result<WebFile, FileSystemError> {
        let f = async || -> Result<WebFile, JsValue> {
            let js_file: FileWrapper = self
                .directory
                .openFile(str::from_utf8(name).map_err(|_| TypeError::new("name was not UTF-8"))?)
                .await?
                .dyn_into()?;
            Ok(WebFile { file: js_file })
        };
        f().await.map_err(to_filesystem_error)
    }
}

/// A provider for the [`File`] trait which delegates to web APIs
pub struct WebFile {
    file: FileWrapper,
}

impl File for WebFile {
    async fn read_all(&mut self) -> Result<Vec<u8>, FileSystemError> {
        let f = async || -> Result<Vec<u8>, JsValue> {
            let data: Uint8Array = self.file.readAll().await?.dyn_into()?;
            let mut out = vec![0u8; data.length() as usize];
            data.copy_to(&mut out);
            Ok(out)
        };
        f().await.map_err(to_filesystem_error)
    }

    async fn read_segment(
        &mut self,
        offset: Offset,
        dest: &mut [u8],
    ) -> Result<usize, FileSystemError> {
        let mut f = async || -> Result<usize, JsValue> {
            assert!(offset.0 <= 2u64.pow(53), "offset not representable as f64");
            #[allow(clippy::cast_precision_loss)]
            let offset = offset.0 as f64;
            assert!(
                dest.len() as u64 <= 2u64.pow(53),
                "length not representable as f64"
            );
            #[allow(clippy::cast_precision_loss)]
            let length = dest.len() as f64;
            let data: Uint8Array = self.file.readSegment(offset, length).await?.dyn_into()?;
            let bytes_read = data.length() as usize;
            data.copy_to(&mut dest[0..bytes_read]);
            Ok(bytes_read)
        };
        f().await.map_err(to_filesystem_error)
    }
}
