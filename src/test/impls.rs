use crate::{
    file_system::FSGenerics,
    test::directory::{TestRepoDirectory, TestRepoFile},
};

pub struct TestGenerics;
impl FSGenerics for TestGenerics {
    type File = TestRepoFile;
    type Directory = TestRepoDirectory;
}
