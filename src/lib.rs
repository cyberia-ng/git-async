#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod directory;
pub mod error;
pub mod object;
pub mod reference;
pub mod repo;

#[cfg(test)]
mod test {
    pub mod repo;
}
