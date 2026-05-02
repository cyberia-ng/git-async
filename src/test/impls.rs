use crate::{
    file_system::FileSystem,
    test::directory::{TestRepoDirectory, TestRepoFile},
};

pub struct TestFileSystem;
impl FileSystem for TestFileSystem {
    type File = TestRepoFile;
    type Directory = TestRepoDirectory;
}
