use crate::{
    test::directory::{TestRepoDirectory, TestRepoFile},
    traits::AllGenerics,
};

pub struct TestGenerics;
impl AllGenerics for TestGenerics {
    type File = TestRepoFile;
    type Directory = TestRepoDirectory;
}
