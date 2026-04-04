use super::PackObjectLocation;
use crate::{
    directory::{DirEntry, Directory, File},
    error::{Error, GResult},
    object::ObjectId,
    repo::Repo,
};
use alloc::vec::Vec;
use core::cmp::Ordering;

async fn find_object<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<PackObjectLocation>> {
    let pack_dir = repo
        .git_dir
        .open_subdir(b"objects")
        .await?
        .open_subdir(b"pack")
        .await?;
    let idx_filenames: Vec<Vec<u8>> = pack_dir
        .list_dir()
        .await?
        .into_iter()
        .filter_map(|dirent| -> Option<Vec<u8>> {
            use DirEntry::*;
            let name = if let File(name) = dirent {
                Some(name)
            } else {
                None
            }?;
            if name.get(0..5) == Some(b"pack-")
                && name.get((name.len() - 4)..name.len()) == Some(b".idx")
            {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    let mut location: Option<PackObjectLocation> = None;
    for idx in idx_filenames {
        let mut idx_file = pack_dir.open_file(&idx).await?;
        if let Some((obj_idx, total_objects)) = find_object_idx(&mut idx_file, id).await? {
            let offset = get_obj_packfile_offset(&mut idx_file, obj_idx, total_objects).await?;
            let mut pack_file_name = idx.strip_suffix(b".idx").unwrap().to_vec();
            pack_file_name.extend_from_slice(b".pack");
            location = Some(PackObjectLocation {
                pack_file_name,
                offset,
            });
            break;
        }
    }
    Ok(location)
}

async fn find_object_idx<F: File>(file: &mut F, id: ObjectId) -> GResult<Option<(u32, u32)>> {
    let mut buf = [0u8; 8];
    file.read_segment(0x0, &mut buf).await?;
    if buf != [0xff, b't', b'O', b'c', 0, 0, 0, 2] {
        return Err(Error::UnsupportedIndexVersion);
    }
    let mut buf = [0u8; 4];
    let fanout_offset: u64 = 0x08;

    file.read_segment(fanout_offset + 4 * 0xff, &mut buf)
        .await?;
    let total_objects = u32::from_be_bytes(buf);

    let first_oid_byte = id.0[0];
    let fanout_oid_offset: u64 = fanout_offset + 4 * u64::from(first_oid_byte);

    let prev_fanout_entry = if first_oid_byte == 0 {
        0
    } else {
        file.read_segment(fanout_oid_offset - 4, &mut buf).await?;
        u32::from_be_bytes(buf)
    };

    file.read_segment(fanout_oid_offset, &mut buf).await?;
    let fanout_entry = u32::from_be_bytes(buf);

    let ids_offset = fanout_offset + 4 * 256;
    let mut buf = [0u8; 20];
    let mut lower_idx = prev_fanout_entry; // inclusive
    let mut upper_idx = fanout_entry; // exclusive
    let mut obj_idx: Option<u32> = None;
    while obj_idx.is_none() && lower_idx < upper_idx {
        let mid_idx: u32 = (lower_idx + upper_idx) / 2;
        let mid_offset: u64 = u64::from(mid_idx) * 20 + ids_offset;
        file.read_segment(mid_offset, &mut buf).await?;
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
    Ok(obj_idx.map(|idx| (idx, total_objects)))
}

async fn get_obj_packfile_offset<F: File>(
    idx_file: &mut F,
    obj_idx: u32,
    total_objects: u32,
) -> GResult<u64> {
    let fanout: u64 = 0x8;
    let object_ids: u64 = fanout + 4 * 256;
    let crc_table: u64 = object_ids + u64::from(total_objects) * 20;
    let short_table: u64 = crc_table + u64::from(total_objects) * 4;
    let mut buf = [0u8; 4];
    let short_entry: u64 = short_table + u64::from(obj_idx) * 4;
    idx_file.read_segment(short_entry, &mut buf).await?;
    let packfile_offset_short = u32::from_be_bytes(buf);
    if packfile_offset_short & 0x80000000 != 0 {
        let long_table_idx: u32 = packfile_offset_short & 0x7fffffff;
        let long_table_offset: u64 = short_table + 4 * u64::from(total_objects);
        let long_entry: u64 = long_table_offset + 8 * u64::from(long_table_idx);
        let mut buf = [0u8; 8];
        idx_file.read_segment(long_entry, &mut buf).await?;
        let packfile_offset_long = u64::from_be_bytes(buf);
        Ok(packfile_offset_long)
    } else {
        Ok(u64::from(packfile_offset_short))
    }
}

#[cfg(test)]
mod tests {
    use crate::test::helpers::{make_file, make_packfile_repo};
    use futures::executor::block_on;
    use hex_literal::hex;
    use rand_core::{Rng, SeedableRng};
    use rand_pcg::Pcg32;
    use std::io::Write;

    use super::*;

    #[test]
    fn test_find_object_idx() {
        let repo = make_packfile_repo().unwrap();
        let mut idx_file = repo
            .pack_idx_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let obj_idx = block_on(find_object_idx(
            &mut idx_file,
            ObjectId(hex!("78dc5b70bd81aa46ec7dfce87a69826e354a916b")),
        ))
        .unwrap();
        assert_eq!(obj_idx, Some((1, 4)));
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

    #[test]
    fn test_get_obj_packfile_offset_normal() {
        let repo = make_packfile_repo().unwrap();
        let mut idx_file = repo
            .pack_idx_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let pack_offset = block_on(get_obj_packfile_offset(&mut idx_file, 1, 4)).unwrap();
        assert_eq!(pack_offset, 0x0c);
    }

    #[ignore]
    #[test]
    fn test_get_obj_packfile_offset_huge() {
        // This test takes a long time and requires many GiB of disk space. Run it by passing
        // --ignored to cargo test
        let repo = make_packfile_repo().unwrap();
        let mut huge_file = make_file(&repo, "a-huge-file").unwrap();
        let mut buf = vec![0u8; 64 * 1048576];
        let mut rng = Pcg32::seed_from_u64(0);
        for _ in 0..(4096 / 64) {
            rng.fill_bytes(&mut buf);
            huge_file.write_all(&buf).unwrap();
        }
        huge_file.flush().unwrap();

        let metadata = huge_file.metadata().unwrap();
        assert_eq!(metadata.len(), 4096 * 1048576);
        repo.run_git(["add", "a-huge-file"]).unwrap();

        repo.commit(
            "another commit",
            "a user",
            "an-email-address",
            "2000-01-01T00:00:00Z",
        )
        .unwrap();
        let head_id = repo
            .run_git(["rev-parse", "HEAD"])
            .unwrap()
            .trim_ascii_end()
            .to_vec();
        assert_eq!(head_id, b"2b9789abe6006287ee2e70570b23ea421084de08");
        repo.run_git(["gc"]).unwrap();
        let mut idx_file = repo
            .pack_idx_file(b"5f4c101929db231a8ae42b13a04abc8aad107a7d")
            .unwrap();
        let pack_offset = block_on(get_obj_packfile_offset(&mut idx_file, 5, 6)).unwrap();
        assert_eq!(pack_offset, 0x0100140150);
    }
}
