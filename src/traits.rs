use crate::file_system::{Directory, File};

pub trait AllGenerics: 'static {
    type File: File;
    type Directory: Directory<Self::File>;
}
