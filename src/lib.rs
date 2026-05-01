//! An async-first library for reading git repositories
//!
//! The library is generic over filesystem operations, so consumers must
//! implement the necessary traits for files and directories. See the
//! [`file_system`] module for further details.
//!
//! The main entry point is the [`Repo`] object, which represents a git
//! repository. Refs and objects are looked up via methods on [`Repo`].

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
