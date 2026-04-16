use core::marker::PhantomData;

use crate::{
    file_system::{Directory, File},
    sync::SharedCell,
};

pub trait AllGenerics: 'static {
    type File: File;
    type Directory: Directory<Self::File>;
    type SharedCell<T: 'static>: SharedCell<T>;
}

#[derive(Debug)]
pub struct Never<T = ()>(PhantomData<T>);
impl<T> Never<T> {
    pub(crate) fn new() -> Self {
        Self(PhantomData)
    }
}
impl<T> Clone for Never<T> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl AllGenerics for Never {
    type File = Never;
    type Directory = Never;
    type SharedCell<T: 'static> = Never<T>;
}
