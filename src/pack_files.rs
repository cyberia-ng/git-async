use core::cmp::Ordering;

use crate::{
    directory::{DirEntry, Directory, File},
    error::{Error, GResult},
    object::ObjectId,
    repo::Repo,
};
use alloc::vec::Vec;

struct PackObjectLocation {
    pack_id: Vec<u8>,
    offset: u64,
}

pub async fn find_object<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<PackObjectLocation> {
    let pack_dir = repo
        .git_dir
        .open_subdir(b"objects")
        .await?
        .open_subdir(b"pack")
        .await?;
    let pack_ids: Vec<&[u8]> = pack_dir
        .list_dir()
        .await?
        .iter()
        .filter_map(|dirent| -> Option<&[u8]> {
            use DirEntry::*;
            let name = if let File(name) = dirent {
                Some(name)
            } else {
                None
            }?;
            let s = name.strip_prefix(b"pack-")?;
            let s = s.strip_suffix(b".idx")?;
            Some(s)
        })
        .collect();
    todo!()
}

pub async fn find_object_idx<F: File>(file: &mut F, id: ObjectId) -> GResult<Option<u32>> {
    let mut buf = [0u8; 4];
    file.read_segment(0x0, &mut buf).await?;
    if buf != [0xff, b't', b'O', b'c'] {
        return Err(Error::UnsupportedIndexVersion);
    }
    file.read_segment(0x4, &mut buf).await?;
    if buf != [0, 0, 0, 2] {
        return Err(Error::UnsupportedIndexVersion);
    }
    let fanout_offset: u64 = 0x08;
    let first_oid_byte = id.0[0];
    let fanout_oid_offset: u64 = u64::from(fanout_offset) + 4 * u64::from(first_oid_byte);

    let prev_fanout_entry = if first_oid_byte == 0 {
        0
    } else {
        file.read_segment(fanout_oid_offset - 4, &mut buf).await?;
        u32::from_be_bytes(buf.clone())
    };

    file.read_segment(fanout_oid_offset, &mut buf).await?;
    let fanout_entry = u32::from_be_bytes(buf.clone());

    let ids_offset = fanout_offset + 4 * 256;
    let mut buf = [0u8; 20];
    let mut lower_idx = prev_fanout_entry; // inclusive
    let mut upper_idx = fanout_entry; // exclusive
    let mut obj_idx: Option<u32> = None;
    while obj_idx.is_none() && lower_idx < upper_idx {
        let mid_idx: u32 = (lower_idx + upper_idx) / 2;
        let mid_offset: u64 = u64::from(mid_idx) * 20 + ids_offset;
        file.read_segment(mid_offset.into(), &mut buf).await?;
        match buf.cmp(&id.0) {
            Ordering::Equal => {
                obj_idx = Some(mid_idx);
            }
            Ordering::Less => {
                lower_idx = mid_idx;
            }
            Ordering::Greater => {
                upper_idx = mid_idx;
            }
        }
    }
    Ok(obj_idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        directory::Directory,
        error::GResult,
        test::repo::{TestRepo, TestRepoFile},
    };
    use futures::executor::block_on;
    use hex_literal::hex;
    use std::{
        fs::OpenOptions,
        io::Write,
        process::{Command, Stdio},
    };

    #[test]
    fn test_find_object_idx() {
        // This test is sensitive to git's packfile algorithm.
        // Expected data was generated with git 2.52.0.
        let repo = TestRepo::new().unwrap();
        repo.set_user("a user", "an-email-address").unwrap();
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .open(repo.location.path().join("a-file"))
            .unwrap();
        f.flush().unwrap();
        repo.run_git(["add", "a-file"]).unwrap();
        let mut p = Command::new("git")
            .current_dir(repo.location.path())
            .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
            .args(["commit", "-m", "a commit"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let status = p.wait().unwrap();
        assert!(status.success());
        let head_id = repo
            .run_git(["rev-parse", "HEAD"])
            .unwrap()
            .trim_ascii_end()
            .to_vec();
        assert_eq!(head_id, b"78dc5b70bd81aa46ec7dfce87a69826e354a916b");
        repo.run_git(["repack"]).unwrap();
        let mut idx_file = block_on((async || -> GResult<TestRepoFile> {
            Ok(repo
                .repo()
                .git_dir
                .open_subdir(b"objects")
                .await?
                .open_subdir(b"pack")
                .await?
                .open_file(b"pack-2692754bdea34cf95fac0765d24ef49e53188be3.idx")
                .await?)
        })())
        .unwrap();
        let obj_idx = block_on(find_object_idx(
            &mut idx_file,
            ObjectId(hex!("78dc5b70bd81aa46ec7dfce87a69826e354a916b")),
        ))
        .unwrap();
        assert_eq!(obj_idx, Some(1));
        let null_obj_idx = block_on(find_object_idx(
            &mut idx_file,
            ObjectId(hex!("0000000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert_eq!(null_obj_idx, None);
        let similar_obj_idx = block_on(find_object_idx(
            &mut idx_file,
            ObjectId(hex!("7800000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert_eq!(similar_obj_idx, None);
    }
}
