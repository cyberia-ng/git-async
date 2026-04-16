use crate::traits::Never;
use alloc::rc::Rc;
use core::{
    cell::RefCell,
    ops::{Deref, DerefMut},
};
#[cfg(feature = "serde")]
use serde::Serialize;

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum SharedCellError {
    LockPoisoned,
    Borrowed,
}

pub trait SharedCell<Inner: ?Sized + 'static>: Sized + Clone {
    type Guard<'a>: Deref<Target = Inner>
    where
        Self: 'a;
    type MutGuard<'a>: DerefMut<Target = Inner>
    where
        Self: 'a;

    fn new(value: Inner) -> Self;

    fn get<'a>(&'a self) -> impl Future<Output = Result<Self::Guard<'a>, SharedCellError>>;

    fn get_mut<'a>(&'a self) -> impl Future<Output = Result<Self::MutGuard<'a>, SharedCellError>>;
}

pub struct SingleThreadedRcCell<T>(Rc<RefCell<T>>);
impl<T> Clone for SingleThreadedRcCell<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static> SharedCell<T> for SingleThreadedRcCell<T> {
    type Guard<'a> = core::cell::Ref<'a, T>;
    type MutGuard<'a> = core::cell::RefMut<'a, T>;

    fn new(value: T) -> Self {
        Self(Rc::new(RefCell::new(value)))
    }

    async fn get<'a>(&'a self) -> Result<Self::Guard<'a>, SharedCellError> {
        self.0.try_borrow().map_err(|_| SharedCellError::Borrowed)
    }

    async fn get_mut<'a>(&'a self) -> Result<Self::MutGuard<'a>, SharedCellError> {
        self.0
            .try_borrow_mut()
            .map_err(|_| SharedCellError::Borrowed)
    }
}

impl<T: 'static> SharedCell<T> for Never<T> {
    type Guard<'a> = Never<T>;
    type MutGuard<'a> = Never<T>;

    fn new(_value: T) -> Self {
        Self::new()
    }

    async fn get<'a>(&'a self) -> Result<Self::Guard<'a>, SharedCellError> {
        unreachable!()
    }
    async fn get_mut<'a>(&'a self) -> Result<Self::MutGuard<'a>, SharedCellError> {
        unreachable!()
    }
}

impl<T> Deref for Never<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unreachable!()
    }
}

impl<T> DerefMut for Never<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unreachable!()
    }
}

#[cfg(test)]
mod tests {}
