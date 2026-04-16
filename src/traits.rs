use crate::file_system::{Directory, File};

pub trait AllGenerics {
    type File: File;
    type Directory: Directory<Self::File>;
}
