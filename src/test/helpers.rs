use crate::test::repo::TestRepo;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
};

pub fn make_file(repo: &TestRepo, file_name: &str) -> io::Result<fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(repo.location.path().join(file_name))
}

pub fn make_basic_repo() -> io::Result<TestRepo> {
    let repo = TestRepo::new()?;
    let mut f = make_file(&repo, "a-file")?;
    f.flush()?;
    repo.run_git(["add", "--all"])?;
    repo.commit(
        "a commit",
        "a user",
        "an-email-address",
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00Z",
    )?;
    Ok(repo)
}

pub fn make_packfile_repo() -> io::Result<TestRepo> {
    // This test helper is sensitive to git's packfile algorithm.
    // Expected data was generated with git 2.52.0.
    let repo = make_basic_repo()?;
    let head_id = repo
        .run_git(["rev-parse", "HEAD"])
        .unwrap()
        .trim_ascii_end()
        .to_vec();
    assert_eq!(head_id, b"78dc5b70bd81aa46ec7dfce87a69826e354a916b");
    repo.run_git(["gc"])?;
    Ok(repo)
}
