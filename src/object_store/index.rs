use crate::{
    directory::{File, Offset},
    error::{Error, GResult},
    object::ObjectId,
};
use core::cmp::Ordering;

pub(crate) async fn find_object_in_pack_index<F: File>(
    idx_file: &mut F,
    id: ObjectId,
) -> GResult<Option<Offset>> {
    if let Some((obj_idx, total_objects)) = find_object_idx(idx_file, id).await? {
        let offset = get_obj_packfile_offset(idx_file, obj_idx, total_objects).await?;
        Ok(Some(offset))
    } else {
        Ok(None)
    }
}

async fn find_object_idx<F: File>(file: &mut F, id: ObjectId) -> GResult<Option<(u32, u32)>> {
    let mut buf = [0u8; 8];
    file.read_segment(Offset(0), &mut buf).await?;
    if buf != [0xff, b't', b'O', b'c', 0, 0, 0, 2] {
        return Err(Error::UnsupportedIndexVersion);
    }
    let mut buf = [0u8; 4];
    let fanout: Offset = Offset(0x08);

    file.read_segment(fanout + 4 * 0xff, &mut buf).await?;
    let total_objects = u32::from_be_bytes(buf);

    let first_oid_byte = id.id()[0];
    let fanout_oid: Offset = fanout + 4 * u64::from(first_oid_byte);

    let prev_fanout_entry = if first_oid_byte == 0 {
        0
    } else {
        file.read_segment(fanout_oid - 4, &mut buf).await?;
        u32::from_be_bytes(buf)
    };

    file.read_segment(fanout_oid, &mut buf).await?;
    let fanout_entry = u32::from_be_bytes(buf);

    let ids_offset = fanout + 4 * 256;
    let mut buf = [0u8; 20];
    let mut lower_idx = prev_fanout_entry; // inclusive
    let mut upper_idx = fanout_entry; // exclusive
    let mut obj_idx: Option<u32> = None;
    while obj_idx.is_none() && lower_idx < upper_idx {
        let mid_idx: u32 = (lower_idx + upper_idx) / 2;
        let mid_offset: Offset = ids_offset + u64::from(mid_idx) * 20;
        file.read_segment(mid_offset, &mut buf).await?;
        match buf.cmp(id.id()) {
            Ordering::Equal => {
                obj_idx = Some(mid_idx);
            }
            Ordering::Less => {
                lower_idx = mid_idx + 1;
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
) -> GResult<Offset> {
    let fanout: Offset = Offset(0x8);
    let object_ids: Offset = fanout + 4 * 256;
    let crc_table: Offset = object_ids + u64::from(total_objects) * 20;
    let short_table: Offset = crc_table + u64::from(total_objects) * 4;
    let mut buf = [0u8; 4];
    let short_entry: Offset = short_table + u64::from(obj_idx) * 4;
    idx_file.read_segment(short_entry, &mut buf).await?;
    let packfile_offset_short = u32::from_be_bytes(buf);
    if packfile_offset_short & 0x80000000 != 0 {
        let long_table_idx: u32 = packfile_offset_short & 0x7fffffff;
        let long_table: Offset = short_table + 4 * u64::from(total_objects);
        let long_entry: Offset = long_table + 8 * u64::from(long_table_idx);
        let mut buf = [0u8; 8];
        idx_file.read_segment(long_entry, &mut buf).await?;
        let packfile_offset_long = u64::from_be_bytes(buf);
        Ok(Offset(packfile_offset_long))
    } else {
        Ok(Offset(u64::from(packfile_offset_short)))
    }
}

#[cfg(test)]
mod tests {
    use crate::test::helpers::{get_pack_id, make_basic_repo, make_file, make_packfile_repo};
    use futures::executor::block_on;
    use hex_literal::hex;
    use rand_core::{Rng, SeedableRng};
    use rand_pcg::Pcg32;
    use std::io::Write;

    use super::*;

    #[test]
    fn test_find_object_idx() {
        let repo = make_packfile_repo().unwrap();
        let pack_id = get_pack_id(&repo).unwrap();
        let mut idx_file = repo.pack_idx_file(&pack_id).unwrap();
        let obj_idx = block_on(find_object_idx(
            &mut idx_file,
            ObjectId::new(hex!("78dc5b70bd81aa46ec7dfce87a69826e354a916b")),
        ))
        .unwrap();
        assert!(obj_idx.is_some());
        let null_obj_idx = block_on(find_object_idx(
            &mut idx_file,
            ObjectId::new(hex!("0000000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert_eq!(null_obj_idx, None);
        let similar_obj_idx = block_on(find_object_idx(
            &mut idx_file,
            ObjectId::new(hex!("7800000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert_eq!(similar_obj_idx, None);
    }

    #[test]
    fn test_get_obj_packfile_offset_normal() {
        let repo = make_packfile_repo().unwrap();
        let pack_id = get_pack_id(&repo).unwrap();
        let mut idx_file = repo.pack_idx_file(&pack_id).unwrap();
        let (object_idx, total_objects) = block_on(find_object_idx(
            &mut idx_file,
            ObjectId::new(hex!("78dc5b70bd81aa46ec7dfce87a69826e354a916b")),
        ))
        .unwrap()
        .unwrap();
        block_on(get_obj_packfile_offset(
            &mut idx_file,
            object_idx,
            total_objects,
        ))
        .unwrap();
    }

    #[ignore]
    #[test]
    fn test_get_obj_packfile_offset_huge() {
        // This test takes a long time and requires many GiB of disk space. Run it by passing
        // --ignored to cargo test
        let repo = make_basic_repo().unwrap();
        let mut buf = vec![0u8; 1048576];
        let mut rng = Pcg32::seed_from_u64(0);

        let mut huge_file_1 = make_file(&repo, "a-huge-file").unwrap();
        for _ in 0..2048 {
            rng.fill_bytes(&mut buf);
            huge_file_1.write_all(&buf).unwrap();
        }
        huge_file_1.flush().unwrap();
        let mut huge_file_2 = make_file(&repo, "another-huge-file").unwrap();
        for _ in 0..2048 {
            rng.fill_bytes(&mut buf);
            huge_file_2.write_all(&buf).unwrap();
        }
        huge_file_2.flush().unwrap();

        let metadata_1 = huge_file_1.metadata().unwrap();
        assert_eq!(metadata_1.len(), 2048 * 1048576);
        let metadata_2 = huge_file_1.metadata().unwrap();
        assert_eq!(metadata_2.len(), 2048 * 1048576);
        repo.run_git(["add", "a-huge-file"]).unwrap();
        repo.run_git(["add", "another-huge-file"]).unwrap();

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
        assert_eq!(head_id, b"7e352726d6addfb0da5e3990393975188c5625ab");
        let expected_blob_id_another_huge_file =
            ObjectId::new(hex!("ead5be8e71f3cb2e585e14436087fd84119dd354"));
        repo.run_git(["gc"]).unwrap();
        let pack_file_id = get_pack_id(&repo).unwrap();
        let mut idx_file = repo.pack_idx_file(&pack_file_id).unwrap();
        let (object_offset, total_objects) = block_on(find_object_idx(
            &mut idx_file,
            expected_blob_id_another_huge_file,
        ))
        .unwrap()
        .unwrap();
        let pack_offset = block_on(get_obj_packfile_offset(
            &mut idx_file,
            object_offset,
            total_objects,
        ))
        .unwrap();
        assert!(pack_offset.0 >= 0x80000000);
    }
}
