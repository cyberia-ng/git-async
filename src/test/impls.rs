use crate::{
    test::{
        directory::{TestRepoDirectory, TestRepoFile},
        lock::StdLock,
    },
    traits::AllGenerics,
};

pub struct TestGenerics;
impl AllGenerics for TestGenerics {
    type File = TestRepoFile;
    type Directory = TestRepoDirectory;
    type SharedCell<T: 'static> = StdLock<T>;
}
