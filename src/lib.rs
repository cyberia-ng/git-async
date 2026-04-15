#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod error;
pub mod file_system;
pub mod object;
mod object_store;
mod parsing;
pub mod reference;
mod repo;
#[cfg(feature = "serde")]
mod serde;

pub use repo::Repo;

#[cfg(test)]
mod test {
    pub(crate) mod helpers;
    pub(crate) mod repo;
}
