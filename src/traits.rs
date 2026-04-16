use crate::file_system::{Directory, File};

pub trait AllGenerics {
    type File: File;
    type Directory: Directory<Self::File>;
}

#[derive(Debug, Clone)]
pub struct Noop(
    pub(crate) (), // Prevent outside construction
);

impl AllGenerics for Noop {
    type File = Noop;
    type Directory = Noop;
}
