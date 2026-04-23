//! This is a crate :)

#![cfg_attr(doc, warn(missing_docs))]
#![cfg_attr(not(test), no_std)]
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
#[cfg(feature = "serde")]
mod serde;
mod subslice_range;
pub mod traits;

pub use repo::Repo;

#[cfg(test)]
mod test;
