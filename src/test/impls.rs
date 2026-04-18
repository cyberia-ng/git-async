use alloc::rc::Rc;

use crate::{
    test::directory::{TestRepoDirectory, TestRepoFile},
    traits::AllGenerics,
};

pub struct TestGenerics;
impl AllGenerics for TestGenerics {
    type File = TestRepoFile;
    type Directory = TestRepoDirectory;
    type SharedRef<T: 'static> = Rc<T>;
}
