use crate::test::repo::TestRepo;
use std::{
    fs::{self, OpenOptions, read_dir, remove_file},
    io::{self, Write},
    os::unix::ffi::OsStrExt,
};

pub fn make_file(repo: &TestRepo, file_name: impl AsRef<std::path::Path>) -> io::Result<fs::File> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(repo.location.path().join(file_name))?;
    f.flush()?;
    Ok(f)
}

pub fn make_basic_repo() -> io::Result<TestRepo> {
    let repo = TestRepo::new()?;
    make_file(&repo, "a-file")?;
    repo.run_git(["add", "--all"])?;
    repo.commit(
        "a commit",
        "a user",
        "an-email-address",
        "2000-01-01T00:00:00Z",
    )?;
    repo.tag_annotated(
        "a-fat-tag",
        "HEAD",
        "a tag",
        "a user",
        "an-email-address",
        "2000-01-01T00:00:00Z",
    )?;
    make_file(&repo, "another-file")?;
    repo.run_git(["add", "--all"])?;
    repo.run_git(["stash"])?;
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
    repo.run_git(["pack-refs", "--all"])?;
    let mut heads_entries = read_dir(repo.location.path().join(".git").join("refs").join("heads"))?;
    assert!(heads_entries.next().is_none());
    let mut tags_entries = read_dir(repo.location.path().join(".git").join("refs").join("tags"))?;
    assert!(tags_entries.next().is_none());
    Ok(repo)
}

pub fn get_pack_id(repo: &TestRepo) -> io::Result<Vec<u8>> {
    let pack_dir_entries = read_dir(
        repo.location
            .path()
            .join(".git")
            .join("objects")
            .join("pack"),
    )?
    .collect::<Result<Vec<_>, _>>()?;
    let mut pack_ids = pack_dir_entries
        .iter()
        .filter_map(|entry| {
            entry
                .file_name()
                .as_bytes()
                .strip_prefix(b"pack-")
                .and_then(|s| s.strip_suffix(b".idx"))
                .map(<[u8]>::to_vec)
        })
        .collect::<Vec<_>>();
    assert_eq!(pack_ids.len(), 1);

    let pack_id = pack_ids.pop().unwrap();
    let idx_file_expected_path = repo
        .location
        .path()
        .join(".git")
        .join("objects")
        .join("pack")
        .join(format!("pack-{}.idx", str::from_utf8(&pack_id).unwrap()));
    assert!(
        OpenOptions::new()
            .read(true)
            .open(idx_file_expected_path)
            .is_ok()
    );
    Ok(pack_id)
}

pub fn make_similar_commits(test_repo: &TestRepo) -> io::Result<()> {
    for chr in 0x61..(0x61 + 26) {
        make_file(test_repo, str::from_utf8(&[chr]).unwrap())?;
    }
    test_repo.run_git(["add", "--all"])?;
    test_repo
        .commit(
            "commit 2",
            "a user",
            "an-email-address",
            "2000-01-01T00:00:00Z",
        )
        .unwrap();
    remove_file(test_repo.location.path().join("m"))?;
    test_repo.run_git(["add", "--all"])?;
    test_repo.commit(
        "commit 3",
        "a user",
        "an-email-address",
        "2000-01-01T00:00:00Z",
    )?;
    remove_file(test_repo.location.path().join("t"))?;
    test_repo.run_git(["add", "--all"])?;
    test_repo.commit(
        "commit 4",
        "a user",
        "an-email-address",
        "2000-01-01T00:00:00Z",
    )?;
    Ok(())
}
