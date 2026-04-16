use crate::sync::{SharedCell, SharedCellError};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct StdLock<T>(Arc<Mutex<T>>);
impl<T> Clone for StdLock<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static> SharedCell<T> for StdLock<T> {
    type Guard<'a> = MutexGuard<'a, T>;
    type MutGuard<'a> = MutexGuard<'a, T>;

    fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }

    async fn get<'a>(&'a self) -> Result<Self::Guard<'a>, SharedCellError> {
        self.0.lock().map_err(|_| SharedCellError::LockPoisoned)
    }

    async fn get_mut<'a>(&'a self) -> Result<Self::MutGuard<'a>, SharedCellError> {
        self.0.lock().map_err(|_| SharedCellError::LockPoisoned)
    }
}

fn _foo<T: Send + Sync>(_val: T) {}

fn _bar<T: Send>(_val: StdLock<T>) {
    _foo(_val)
}
