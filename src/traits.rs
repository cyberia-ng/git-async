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
pub struct Detached<T = ()>(PhantomData<T>);
impl<T> Detached<T> {
    pub(crate) fn new() -> Self {
        Self(PhantomData)
    }
}
impl<T> Clone for Detached<T> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl AllGenerics for Detached {
    type File = Detached;
    type Directory = Detached;
    type SharedCell<T: 'static> = Detached<T>;
}
