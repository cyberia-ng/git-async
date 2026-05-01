use crate::{
    error::{Error, GResult},
    file_system::{File, Offset},
    object::ObjectId,
};
use alloc::{vec, vec::Vec};
use core::cmp::Ordering;

#[derive(Clone)]
pub(crate) struct FanoutTable {
    fanout: [u32; 256],
}

impl FanoutTable {
    pub(crate) async fn load<F: File>(file: &mut F) -> GResult<Self> {
        let mut buf = [0u8; 4 + 4 + 256 * 4];
        let read_size = file.read_segment(Offset(0), &mut buf).await?;
        if read_size != buf.len() {
            return Err(Error::CorruptIndexFile);
        }
        if buf[0..8] != [0xff, b't', b'O', b'c', 0, 0, 0, 2] {
            return Err(Error::UnsupportedIndexVersion);
        }
        let mut fanout = [0u32; 256];
        for (entry_bytes, entry) in buf[8..].chunks(4).zip(fanout.iter_mut()) {
            *entry = u32::from_be_bytes(entry_bytes.try_into().unwrap());
        }
        Ok(Self { fanout })
    }

    pub(crate) fn entry(&self, prefix: u8) -> u32 {
        self.fanout[usize::from(prefix)]
    }

    pub(crate) fn total_objects(&self) -> u32 {
        *self.fanout.last().unwrap()
    }
}

#[derive(Clone)]
pub(crate) struct ShortOffsetTable {
    table: Vec<u8>,
}

impl ShortOffsetTable {
    pub(crate) fn offset_of_table(total_objects: u32) -> Offset {
        Offset(
            4 // header
            + 4 // version
            + 256 * 4 // fanout
            + u64::from(total_objects) * 20 // object IDs
            + u64::from(total_objects) * 4, // CRCs
        )
    }

    pub(crate) async fn load<F: File>(file: &mut F, total_objects: u32) -> GResult<Self> {
        let table_size: usize = usize::try_from(total_objects).unwrap() * 4;
        let mut table = vec![0u8; table_size];
        let read_size = file
            .read_segment(Self::offset_of_table(total_objects), &mut table)
            .await?;
        if read_size < table_size {
            return Err(Error::CorruptIndexFile);
        }
        Ok(Self { table })
    }

    pub(crate) fn entry(&self, object_idx: u32) -> u32 {
        let object_idx: usize = object_idx.try_into().unwrap();
        let entry_bytes = &self.table[(object_idx * 4)..((object_idx + 1) * 4)];
        u32::from_be_bytes(entry_bytes.try_into().unwrap())
    }
}

pub(crate) async fn find_object_in_pack_index<F: File>(
    fanout: &FanoutTable,
    offsets: Option<&ShortOffsetTable>,
    idx_file: &mut F,
    id: ObjectId,
) -> GResult<Option<Offset>> {
    if let Some(obj_idx) = find_object_idx(fanout, idx_file, id).await? {
        let offset =
            get_obj_packfile_offset(offsets, idx_file, obj_idx, fanout.total_objects()).await?;
        Ok(Some(offset))
    } else {
        Ok(None)
    }
}

async fn find_object_idx<F: File>(
    fanout: &FanoutTable,
    idx_file: &mut F,
    id: ObjectId,
) -> GResult<Option<u32>> {
    let first_oid_byte = id.bytes()[0];
    let prev_fanout_entry = if first_oid_byte == 0 {
        0
    } else {
        fanout.entry(first_oid_byte - 1)
    };
    let fanout_entry = fanout.entry(first_oid_byte);

    let ids_offset = Offset(
        4 // magic number
        + 4 // version
        + 256 * 4, // fanout
    );
    let mut buf = [0u8; 20];
    let mut lower_idx = prev_fanout_entry; // inclusive
    let mut upper_idx = fanout_entry; // exclusive
    let mut obj_idx: Option<u32> = None;
    while obj_idx.is_none() && lower_idx < upper_idx {
        let mid_idx: u32 = u32::midpoint(lower_idx, upper_idx);
        let mid_offset: Offset = ids_offset + u64::from(mid_idx) * 20;
        idx_file.read_segment(mid_offset, &mut buf).await?;
        match buf.cmp(id.bytes()) {
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
    Ok(obj_idx)
}

async fn get_obj_packfile_offset<F: File>(
    offset_table: Option<&ShortOffsetTable>,
    idx_file: &mut F,
    obj_idx: u32,
    total_objects: u32,
) -> GResult<Offset> {
    let packfile_offset_short = if let Some(offset_table) = offset_table {
        offset_table.entry(obj_idx)
    } else {
        let entry_offset =
            ShortOffsetTable::offset_of_table(total_objects) + u64::from(obj_idx) * 4;
        let mut buf = [0u8; 4];
        idx_file.read_segment(entry_offset, &mut buf).await?;
        u32::from_be_bytes(buf)
    };
    if packfile_offset_short & 0x8000_0000 != 0 {
        let fanout: Offset = Offset(0x8);
        let object_ids: Offset = fanout + 4 * 256;
        let crc_table: Offset = object_ids + u64::from(total_objects) * 20;
        let short_table: Offset = crc_table + u64::from(total_objects) * 4;
        let long_table_idx: u32 = packfile_offset_short & 0x7fff_ffff;
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
        let fanout = block_on(FanoutTable::load(&mut idx_file)).unwrap();
        let obj_idx = block_on(find_object_idx(
            &fanout,
            &mut idx_file,
            ObjectId::from_bytes(hex!("78dc5b70bd81aa46ec7dfce87a69826e354a916b")),
        ))
        .unwrap();
        assert!(obj_idx.is_some());
        let null_obj_idx = block_on(find_object_idx(
            &fanout,
            &mut idx_file,
            ObjectId::from_bytes(hex!("0000000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert_eq!(null_obj_idx, None);
        let similar_obj_idx = block_on(find_object_idx(
            &fanout,
            &mut idx_file,
            ObjectId::from_bytes(hex!("7800000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert_eq!(similar_obj_idx, None);
    }

    #[test]
    fn test_get_obj_packfile_offset_normal() {
        let repo = make_packfile_repo().unwrap();
        let pack_id = get_pack_id(&repo).unwrap();
        let mut idx_file = repo.pack_idx_file(&pack_id).unwrap();
        let fanout = block_on(FanoutTable::load(&mut idx_file)).unwrap();
        let offsets = block_on(ShortOffsetTable::load(
            &mut idx_file,
            fanout.total_objects(),
        ))
        .unwrap();
        let object_idx = block_on(find_object_idx(
            &fanout,
            &mut idx_file,
            ObjectId::from_bytes(hex!("78dc5b70bd81aa46ec7dfce87a69826e354a916b")),
        ))
        .unwrap()
        .unwrap();
        block_on(get_obj_packfile_offset(
            Some(&offsets),
            &mut idx_file,
            object_idx,
            fanout.total_objects(),
        ))
        .unwrap();
    }

    #[ignore = "takes a long time and requires many GiB of disk space"]
    #[test]
    fn test_get_obj_packfile_offset_huge() {
        const MEGABYTE: usize = 1024 * 1024;
        let repo = make_basic_repo().unwrap();
        let mut buf = vec![0u8; MEGABYTE];
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
        assert_eq!(metadata_1.len(), 2048 * MEGABYTE as u64);
        let metadata_2 = huge_file_1.metadata().unwrap();
        assert_eq!(metadata_2.len(), 2048 * MEGABYTE as u64);
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
            ObjectId::from_bytes(hex!("ead5be8e71f3cb2e585e14436087fd84119dd354"));
        repo.run_git(["gc"]).unwrap();
        let pack_file_id = get_pack_id(&repo).unwrap();
        let mut idx_file = repo.pack_idx_file(&pack_file_id).unwrap();
        let fanout = block_on(FanoutTable::load(&mut idx_file)).unwrap();
        let offsets = block_on(ShortOffsetTable::load(
            &mut idx_file,
            fanout.total_objects(),
        ))
        .unwrap();
        let object_offset = block_on(find_object_idx(
            &fanout,
            &mut idx_file,
            expected_blob_id_another_huge_file,
        ))
        .unwrap()
        .unwrap();
        let pack_offset = block_on(get_obj_packfile_offset(
            Some(&offsets),
            &mut idx_file,
            object_offset,
            fanout.total_objects(),
        ))
        .unwrap();
        assert!(pack_offset.0 >= 0x8000_0000);
    }
}
