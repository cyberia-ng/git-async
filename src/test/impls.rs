use crate::{
    file_system::FileSystem,
    test::directory::{TestRepoDirectory, TestRepoFile},
};

pub struct TestGenerics;
impl FileSystem for TestGenerics {
    type File = TestRepoFile;
    type Directory = TestRepoDirectory;
}
