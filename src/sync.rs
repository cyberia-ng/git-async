use crate::traits::Detached;
use alloc::rc::Rc;
use core::ops::{Deref, DerefMut};

pub trait SharedRef<Inner: 'static>: Sized + Clone + Deref<Target = Inner> {
    fn new(value: Inner) -> Self;
}

impl<T: 'static> SharedRef<T> for Rc<T> {
    fn new(value: T) -> Self {
        Rc::new(value)
    }
}

impl<T: 'static> SharedRef<T> for Detached<T> {
    fn new(_value: T) -> Self {
        Detached::new()
    }
}

impl<T> Deref for Detached<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unreachable!()
    }
}

impl<T> DerefMut for Detached<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unreachable!()
    }
}
