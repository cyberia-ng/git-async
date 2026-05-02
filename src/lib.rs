//! An async-first library for reading git repositories
//!
//! # Usage
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
//! The main entry point is the [`Repo`] object, which represents a git
//! repository. Refs and objects are looked up via methods on [`Repo`].
//!
//! # Example
//! ```
//! let foo: u8 = 0;
//! ```
//!
//! # Caveats
//!
//! - Read only
//! - Diff is slow

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

mod object_store;
mod parsing;
mod repo;
mod subslice_range;

pub use repo::Repo;
pub use repo::RepoConfig;

#[cfg(test)]
mod test;
