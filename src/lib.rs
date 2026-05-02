//! An async-first Rust library for reading git repositories
//!
//! # Usage
//!
//! The main entry point is the [`Repo`] object, which represents a git
//! repository. Refs and objects are looked up via methods on [`Repo`].
//!
//! The library is agnostic as to the async runtime in use, so consumers must
//! implement a couple of traits that provide filesystem operations. See the
//! [`file_system`] module for further details.
//!
//! For example, these could use Tokio, or the web filesystem API using
//! wasm-bindgen's support for transforming JS promises to Rust futures. A dummy
//! implementation could use the Rust standard library's synchronous filesystem
//! operations.
//!
//! A future goal is to provide some standard implementations for commonly-used
//! async runtimes.
//!
//! # Example
//!
//! ```
//! # use git_async::Repo;
//! # use git_async::error::GResult;
//! # use git_async::file_system::{File, Directory, FileSystem, FileSystemError, Offset, DirEntry};
//! # struct MyFile;
//! # impl File for MyFile {
//! #     async fn read_all(&mut self) -> Result<Vec<u8>, FileSystemError> { unimplemented!() }
//! #     async fn read_segment(&mut self, _: Offset, _: &mut [u8]) -> Result<usize, FileSystemError> { unimplemented! () }
//! # }
//! # struct MyDirectory;
//! # impl MyDirectory { fn new(s: &str) -> Self { unimplemented!() } }
//! # impl Clone for MyDirectory { fn clone(&self) -> Self { unimplemented!() } }
//! # impl Directory<MyFile> for MyDirectory {
//! #     async fn open_subdir(&self, _: &[u8]) -> Result<Self, FileSystemError> { unimplemented!() }
//! #     async fn list_dir(&self) -> Result<Vec<DirEntry>, FileSystemError> { unimplemented!() }
//! #     async fn open_file(&self, _: &[u8]) -> Result<MyFile, FileSystemError> { unimplemented!() }
//! # }
//! # struct MyFS;
//! # impl FileSystem for MyFS {
//! #     type Directory = MyDirectory;
//! #     type File = MyFile;
//! # }
//! async fn example() -> GResult<()> {
//!     let repo = Repo::<MyFS>::open(MyDirectory::new("a-repository")).await?;
//!     let head = repo.head().await?;
//!     let commit = head.peel_to_commit(&repo).await?.unwrap();
//!     let message = commit.message();
//!     println!("{}", str::from_utf8(message).unwrap());
//!     Ok(())
//! }
//! ```
//!
//! # Caveats
//!
//! There are a few things this crate cannot (yet) do. They are in scope, so
//! future versions may support them, but for now they are not implemented.
//!
//! - It only supports read operations on git repositories
//! - It ignores the working tree. Only operations involving the actual
//!   repository structure are supported
//! - The diff algorithm is naive and quite slow. I have not looked into how
//!   `git diff` manages to be so fast, but I imagine it uses the packfile delta
//!   encoding somehow to optimize diffing.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(doc, warn(missing_docs))]
#![cfg_attr(not(test), no_std)]
#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::enum_glob_use)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_panics_doc)]
#![cfg_attr(test, allow(clippy::cast_possible_truncation))]

#[cfg(doc)]
extern crate std;

extern crate alloc;

#[cfg(feature = "diff")]
pub mod diff;
pub mod error;
pub mod file_system;
pub mod object;
pub mod reference;
#[cfg(feature = "web")]
pub mod web;

mod object_store;
mod parsing;
mod repo;
mod subslice_range;

pub use repo::Repo;
pub use repo::RepoConfig;

#[cfg(test)]
mod test;
