//! This is a crate :)

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
mod object_store;
mod parsing;
pub mod reference;
mod repo;
mod subslice_range;
pub mod traits;

pub use repo::Repo;

#[cfg(test)]
mod test;
