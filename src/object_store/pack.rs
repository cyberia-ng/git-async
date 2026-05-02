use crate::{
    error::{Error, GResult, IResult, InternalObjectError},
    file_system::{File, Offset},
    object::ObjectId,
    object_store::{
        ObjectSize, ObjectType, index::find_object_in_pack_index, lookup::IndexedPackFile,
    },
};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use miniz_oxide::inflate::{
    TINFLStatus,
    core::{
        DecompressorOxide, decompress,
        inflate_flags::{
            TINFL_FLAG_HAS_MORE_INPUT, TINFL_FLAG_PARSE_ZLIB_HEADER,
            TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
        },
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct PackNegativeOffset(pub u64);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PackObjectType {
    Base(ObjectType),
    OffsetDelta { base_offset_neg: PackNegativeOffset },
    RefDelta { base_id: ObjectId },
}

#[derive(Debug)]
pub(crate) struct PackObject {
    pub body_offset: Offset,
    pub size: ObjectSize,
    // pub body_initial: Vec<u8>,
}

// Git uses three slightly different algorithms for encoding variable-width
// integers in different contexts within the packfile. This is not documented
// anywhere.

fn read_obj_type_size(buf: &[u8]) -> IResult<(usize, PackObjectType, ObjectSize)> {
    // This algorithm is for reading the first part of the packfile object
    // header, which encodes the object type and size.
    let mut pos: usize = 0;
    let mut object_type: Option<PackObjectType> = None;
    let mut obj_size = ObjectSize(0);
    let mut done_accumulating_size = false;
    for buf_byte in buf {
        done_accumulating_size = (0b1000_0000 & *buf_byte) == 0;
        if pos == 0 {
            let obj_type_id = 0b0111_0000 & *buf_byte;
            object_type = Some(match obj_type_id {
                0b0001_0000 => PackObjectType::Base(ObjectType::Commit),
                0b0010_0000 => PackObjectType::Base(ObjectType::Tree),
                0b0011_0000 => PackObjectType::Base(ObjectType::Blob),
                0b0100_0000 => PackObjectType::Base(ObjectType::Tag),
                0b0110_0000 => PackObjectType::OffsetDelta {
                    base_offset_neg: PackNegativeOffset(0),
                },
                0b0111_0000 => PackObjectType::RefDelta {
                    base_id: ObjectId::from_bytes([0; 20]),
                },
                _ => return Err(InternalObjectError::MalformedPackObject),
            });
            let size_bits = 0b0000_1111 & *buf_byte;
            obj_size.0 = size_bits.into();
        } else {
            let size_bits = 0b0111_1111 & *buf_byte;
            let shift: usize = 4 + 7 * (pos - 1);
            obj_size.0 += u64::from(size_bits) << shift;
        }
        pos += 1;
        if done_accumulating_size {
            break;
        }
    }
    assert!(
        done_accumulating_size,
        "buffer was too short to hold varsize"
    );
    Ok((pos, object_type.unwrap(), obj_size))
}

fn read_delta_offset(buf: &[u8]) -> (usize, PackNegativeOffset) {
    // This algorithm is for reading the second part of the packfile object
    // header (in the case of an offset delta object), which encodes the
    // relative negative offset of the delta object's base object
    let mut bytes_read = 0;
    let mut offset = PackNegativeOffset(0);
    let mut done_accumulating_offset = false;
    for (buf_idx, buf_byte) in buf.iter().enumerate() {
        done_accumulating_offset = (0b1000_0000 & *buf_byte) == 0;
        if buf_idx != 0 {
            offset.0 += 1;
        }
        offset.0 <<= 7;
        offset.0 += u64::from(buf_byte & 0b0111_1111);
        bytes_read += 1;
        if done_accumulating_offset {
            break;
        }
    }
    assert!(
        done_accumulating_offset,
        "buffer was too short to hold varsize"
    );
    (bytes_read, offset)
}

fn read_delta_expected_size(buf: &[u8]) -> (usize, ObjectSize) {
    // This algorithm is for reading the expected base object and un-deltified
    // object sizes, which form the header of the decompressed data stream in an
    // offset delta object.
    let mut bytes_read = 0;
    let mut size = ObjectSize(0);
    let mut done_accumulating_size = false;
    let mut shift = 0;
    for buf_byte in buf {
        done_accumulating_size = (0b1000_0000 & *buf_byte) == 0;
        size.0 += u64::from(buf_byte & 0b0111_1111) << shift;
        shift += 7;
        bytes_read += 1;
        if done_accumulating_size {
            break;
        }
    }
    assert!(
        done_accumulating_size,
        "buffer was too short to hold varsize"
    );
    (bytes_read, size)
}

pub(crate) async fn validate_packfile_version<F: File>(pack_file: &mut F) -> GResult<()> {
    let mut buf = [0u8; 8];
    pack_file.read_segment(Offset(0), &mut buf).await?;
    if buf != [b'P', b'A', b'C', b'K', 0, 0, 0, 2] {
        return Err(Error::UnsupportedPackVersion);
    }
    Ok(())
}

async fn read_pack_object_header<F: File>(
    pack_file: &mut F,
    offset: Offset,
) -> IResult<(PackObjectType, PackObject)> {
    let mut buf = [0u8;
        10 // u64::MAX in object size encoding
        + 20 // max(u64::MAX in offset delta encoding, object ID in ref delta)
    ];
    let mut pos: usize = 0;
    let eof_pos = pack_file
        .read_segment(offset + u64::try_from(pos).unwrap(), &mut buf)
        .await?;
    let (bytes_read, mut object_type, obj_size) = read_obj_type_size(&buf)?;
    pos += bytes_read;
    if pos >= eof_pos {
        return Err(Error::CorruptPackFile.into());
    }

    match object_type {
        PackObjectType::Base(..) => {}
        PackObjectType::OffsetDelta {
            ref mut base_offset_neg,
        } => {
            let (bytes_read, offset) = read_delta_offset(&buf[pos..]);
            *base_offset_neg = offset;
            pos += bytes_read;
        }
        PackObjectType::RefDelta { ref mut base_id } => {
            if eof_pos - pos < 20 {
                return Err(Error::CorruptPackFile.into());
            }
            base_id.bytes.copy_from_slice(&buf[pos..(pos + 20)]);
            pos += 20;
        }
    }

    Ok((
        object_type,
        PackObject {
            body_offset: Offset(offset.0 + (pos as u64)),
            size: obj_size,
        },
    ))
}

async fn read_pack_object_body<F: File>(
    pack_file: &mut F,
    object: &PackObject,
) -> IResult<Vec<u8>> {
    let object_size =
        usize::try_from(object.size.0).map_err(|_| InternalObjectError::ObjectTooLarge)?;
    let mut pos = 0;
    let mut compressed_body_buf = [0u8; 512];
    let mut body = vec![0u8; object_size];
    let mut state = Box::<DecompressorOxide>::default();
    let mut out_idx: usize = 0;
    loop {
        use TINFLStatus::*;
        pack_file
            .read_segment(
                object.body_offset + u64::try_from(pos).unwrap(),
                &mut compressed_body_buf,
            )
            .await?;
        let (status, input_read, output_written) = decompress(
            &mut state,
            &compressed_body_buf,
            &mut body,
            out_idx,
            TINFL_FLAG_HAS_MORE_INPUT
                | TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
                | TINFL_FLAG_PARSE_ZLIB_HEADER,
        );
        pos += input_read;
        out_idx += output_written;
        match status {
            Done => break,
            NeedsMoreInput | HasMoreOutput => {}
            _ => {
                return Err(InternalObjectError::PackObjectDecompressError(status));
            }
        }
    }
    Ok(body)
}

pub(crate) async fn form_deltified_chain<F: File>(
    indexed_pack: &mut IndexedPackFile<'_, F>,
    start_offset: Offset,
) -> IResult<(Vec<PackObject>, ObjectType, PackObject)> {
    let mut chain = Vec::new();
    let mut final_object: Option<(ObjectType, PackObject)> = None;
    let mut offset = start_offset;
    while final_object.is_none() {
        let (object_type, object) = read_pack_object_header(&mut indexed_pack.pack, offset).await?;
        match &object_type {
            PackObjectType::OffsetDelta { base_offset_neg } => {
                offset.0 -= base_offset_neg.0;
                chain.push(object);
            }
            PackObjectType::RefDelta { base_id } => {
                let base_offset = find_object_in_pack_index(
                    indexed_pack.fanout,
                    indexed_pack.offsets,
                    &mut indexed_pack.index,
                    *base_id,
                )
                .await?;
                offset = base_offset
                    .ok_or_else(|| InternalObjectError::from(Error::UnexpectedThinPack))?;
                chain.push(object);
            }
            PackObjectType::Base(base_type) => {
                final_object = Some((*base_type, object));
            }
        }
    }
    let (final_type, final_object) = final_object.unwrap();
    Ok((chain, final_type, final_object))
}

fn reconstruct_deltified_object(deltified: &[u8], base: &[u8]) -> Vec<u8> {
    let mut pos: usize = 0;
    let mut reconstructed_body: Vec<u8> = Vec::new();
    let (bytes_read, base_object_size) = read_delta_expected_size(&deltified[pos..]);
    pos += bytes_read;
    debug_assert_eq!(base_object_size.0, base.len() as u64, "base size");
    let (bytes_read, reconstructed_body_size) = read_delta_expected_size(&deltified[pos..]);
    pos += bytes_read;
    while pos < deltified.len() {
        let mut instruction = deltified[pos];
        pos += 1;
        if instruction & 0b1000_0000 == 0 {
            // Append
            let size = usize::from(instruction & 0b0111_1111);
            reconstructed_body.extend_from_slice(&deltified[pos..(pos + size)]);
            pos += size;
        } else {
            // Copy
            let mut offset = [0u8; 4];
            let mut size = [0u8; 4];
            for offset_byte in &mut offset {
                if instruction & 1 != 0 {
                    *offset_byte = deltified[pos];
                    pos += 1;
                }
                instruction >>= 1;
            }
            for size_byte in &mut size[..3] {
                if instruction & 1 != 0 {
                    *size_byte = deltified[pos];
                    pos += 1;
                }
                instruction >>= 1;
            }
            let offset = usize::try_from(u32::from_le_bytes(offset)).unwrap();
            let mut size = usize::try_from(u32::from_le_bytes(size)).unwrap();
            if size == 0 {
                size = 0x10000;
            }
            reconstructed_body.extend_from_slice(&base[offset..(offset + size)]);
        }
    }
    debug_assert_eq!(
        reconstructed_body_size.0,
        reconstructed_body.len() as u64,
        "reconstructed size"
    );
    reconstructed_body
}

pub(crate) async fn reconstruct_deltified_object_from_chain<F: File>(
    indexed_pack: &mut IndexedPackFile<'_, F>,
    chain: &[PackObject],
    final_object: &PackObject,
) -> IResult<Vec<u8>> {
    let chain_iter = chain.iter().rev();
    let mut reconstructed_body =
        read_pack_object_body(&mut indexed_pack.pack, final_object).await?;
    for pack_object in chain_iter {
        let pack_object_body = read_pack_object_body(&mut indexed_pack.pack, pack_object).await?;
        reconstructed_body = reconstruct_deltified_object(&pack_object_body, &reconstructed_body);
    }
    Ok(reconstructed_body)
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, rename};

    use crate::{
        object::ObjectId,
        object_store::{
            cache::IndexCache,
            lookup::{find_packed_object, lookup},
        },
        repo::RepoConfig,
        test::{
            helpers::{make_basic_repo, make_packfile_repo, make_similar_commits},
            impls::TestFileSystem,
        },
    };
    use futures::executor::block_on;
    use hex_literal::hex;

    use super::*;

    #[test]
    fn read_non_deltified_commit() {
        let test_repo = make_packfile_repo().unwrap();
        let raw_object = block_on(lookup(
            &test_repo.repo(),
            ObjectId::from_hex(b"78dc5b70bd81aa46ec7dfce87a69826e354a916b").unwrap(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(raw_object.object_type, ObjectType::Commit);
        let expected_body = b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a user <an-email-address> 946684800 +0000
committer a user <an-email-address> 946684800 +0000

a commit
";
        assert_eq!(raw_object.body, expected_body);
    }

    #[test]
    fn read_non_deltified_blob() {
        let test_repo = make_packfile_repo().unwrap();
        let raw_object = block_on(lookup(
            &test_repo.repo(),
            ObjectId::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(raw_object.object_type, ObjectType::Blob);
        assert_eq!(raw_object.body, b"");
    }

    #[test]
    fn read_non_deltified_tree() {
        let test_repo = make_packfile_repo().unwrap();
        let raw_object = block_on(lookup(
            &test_repo.repo(),
            ObjectId::from_hex(b"3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb").unwrap(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(raw_object.object_type, ObjectType::Tree);
        let mut expected = Vec::new();
        expected.extend_from_slice(b"100644 a-file\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        assert_eq!(raw_object.body, expected);
    }

    #[test]
    fn read_non_deltified_tag() {
        let test_repo = make_packfile_repo().unwrap();
        let raw_object = block_on(lookup(
            &test_repo.repo(),
            ObjectId::from_hex(b"fbb9ae04dfa95dc527c1e6dde722f9048c5262ef").unwrap(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(raw_object.object_type, ObjectType::Tag);
        assert_eq!(
            raw_object.body,
            b"object 78dc5b70bd81aa46ec7dfce87a69826e354a916b
type commit
tag a-fat-tag
tagger a user <an-email-address> 946684800 +0000

a tag
"
        );
    }

    #[test]
    fn read_deltified_offset_object() {
        let test_repo = make_basic_repo().unwrap();
        make_similar_commits(&test_repo).unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let repo = test_repo.repo();
        let cache = block_on(IndexCache::new(&repo.pack_dir, &RepoConfig::default())).unwrap();
        let (mut pack, offset) = block_on(find_packed_object(
            &repo,
            &cache,
            ObjectId::from_hex(b"7ee3a2eb0ff69340e8a1c962a5b573de1cb9b1f6").unwrap(),
        ))
        .unwrap()
        .unwrap();
        let (object_type, pack_object) =
            block_on(read_pack_object_header(&mut pack.pack, offset)).unwrap();
        assert_eq!(
            object_type,
            PackObjectType::OffsetDelta {
                base_offset_neg: PackNegativeOffset(128)
            }
        );
        let body = block_on(read_pack_object_body(&mut pack.pack, &pack_object)).unwrap();
        assert_eq!(body, hex!("94 06 f7 05 b0 85 01 b3 a2 01 72 01"));
    }

    #[test]
    fn form_deltified_object_chain() {
        let test_repo = make_basic_repo().unwrap();
        make_similar_commits(&test_repo).unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let repo = test_repo.repo();
        let cache = block_on(IndexCache::new(&repo.pack_dir, &RepoConfig::default())).unwrap();
        let (mut pack, offset) = block_on(find_packed_object(
            &repo,
            &cache,
            ObjectId::from_hex(b"9cded1c631096bb2caf71e1f2e0765bf6420d040").unwrap(),
        ))
        .unwrap()
        .unwrap();
        let (chain, _, _) = block_on(form_deltified_chain(&mut pack, offset)).unwrap();
        assert_eq!(chain.len(), 2);
    }

    #[test]
    fn reconstruct_one_object() {
        let mut base_object = vec![0u8; 128 * 1024];
        for (i, item) in base_object.iter_mut().enumerate() {
            *item = (i % u8::MAX as usize) as u8;
        }

        let mut deltified_object = Vec::new();
        let base_object_size_encoded: [u8; _] = [0b1000_0000, 0b1000_0000, 0b0000_1000]; // 128 * 1024
        assert_eq!(
            read_delta_expected_size(&base_object_size_encoded).1,
            ObjectSize(128 * 1024)
        );
        let target_object_size_encoded: [u8; _] = [0b1000_1101, 0b1000_0000, 0b0000_0100]; // 10 + 3 + 0x10000
        assert_eq!(
            read_delta_expected_size(&target_object_size_encoded).1,
            ObjectSize(10 + 3 + 0x10000)
        );

        deltified_object.extend_from_slice(&base_object_size_encoded);
        deltified_object.extend_from_slice(&target_object_size_encoded);

        // Small copy
        let offset_1: u32 = 65;
        let size_1: u32 = 10;
        let instruction_1: [u8; _] = [0b1001_0001, 65, 10];
        deltified_object.extend_from_slice(&instruction_1);

        // Append
        let instruction_2: [u8; _] = [0b0000_0011, 0xc0, 0xff, 0xee];
        deltified_object.extend_from_slice(&instruction_2);

        // Copy with special case size = 0 (interpeted as size = 0x10000)
        let offset_3: u32 = 0x10000;
        let instruction_3: [u8; _] = [0b1000_0100, 0x01];
        deltified_object.extend_from_slice(&instruction_3);

        let reconstructed = reconstruct_deltified_object(&deltified_object, &base_object);

        assert_eq!(reconstructed.len(), 10 + 3 + 0x10000);
        let mut expected = Vec::new();
        expected
            .extend_from_slice(&base_object[(offset_1 as usize)..((offset_1 + size_1) as usize)]);
        expected.extend_from_slice(&[0xc0, 0xff, 0xee]);
        expected.extend_from_slice(&base_object[offset_3 as usize..(offset_3 + 0x10000) as usize]);
        assert_eq!(expected.len(), 10 + 3 + 0x10000);
        assert!(reconstructed == expected);
    }

    #[test]
    fn reconstruct_chained_deltified_object() {
        let test_repo = make_basic_repo().unwrap();
        make_similar_commits(&test_repo).unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let raw_object = block_on(lookup(
            &test_repo.repo(),
            ObjectId::from_hex(b"9cded1c631096bb2caf71e1f2e0765bf6420d040").unwrap(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(raw_object.object_type, ObjectType::Tree);
        let mut expected = Vec::new();
        expected.extend_from_slice(b"100644 a\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        expected.extend_from_slice(b"100644 a-file\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        for c in b'b'..=b'z' {
            if c != b'm' && c != b't' {
                expected.extend_from_slice(b"100644 ");
                expected.push(c);
                expected.push(b'\0');
                expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
            }
        }
        assert_eq!(raw_object.body, expected);
    }

    #[test]
    fn ref_delta() {
        let test_repo = make_packfile_repo().unwrap();
        make_similar_commits(&test_repo).unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let objects_dir = test_repo.location.path().join(".git").join("objects");
        create_dir(objects_dir.join("pack-new")).unwrap();
        let mut git_process = test_repo
            .git_command()
            .current_dir(objects_dir.join("pack-new"))
            .args([
                "pack-objects",
                "--revs",
                "--no-reuse-delta",
                "--all",
                "pack-refdelta",
                // --delta-base-offset is off by default, which is what we want
            ])
            .spawn()
            .unwrap();
        assert!(git_process.wait().unwrap().success());
        rename(objects_dir.join("pack"), objects_dir.join("pack-old")).unwrap();
        rename(objects_dir.join("pack-new"), objects_dir.join("pack")).unwrap();
        assert!(test_repo.run_git(["rev-parse", "HEAD^"]).is_ok());
        let raw_object = block_on(lookup(
            &test_repo.repo(),
            ObjectId::from_hex(b"9cded1c631096bb2caf71e1f2e0765bf6420d040").unwrap(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(raw_object.object_type, ObjectType::Tree);
        let mut expected = Vec::new();
        expected.extend_from_slice(b"100644 a\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        expected.extend_from_slice(b"100644 a-file\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        for c in b'b'..=b'z' {
            if c != b'm' && c != b't' {
                expected.extend_from_slice(b"100644 ");
                expected.push(c);
                expected.push(b'\0');
                expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
            }
        }
        assert_eq!(raw_object.body.len(), expected.len());
        assert_eq!(raw_object.body, expected);
    }

    #[test]
    fn read_object_no_offset_cache() {
        let test_repo = make_packfile_repo().unwrap();
        let repo = block_on(
            RepoConfig::default()
                .index_offset_cache_max(0)
                .open::<TestFileSystem>(test_repo.git_dir()),
        )
        .unwrap();
        let raw_object = block_on(lookup(
            &repo,
            ObjectId::from_hex(b"78dc5b70bd81aa46ec7dfce87a69826e354a916b").unwrap(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(raw_object.object_type, ObjectType::Commit);
        let expected_body = b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a user <an-email-address> 946684800 +0000
committer a user <an-email-address> 946684800 +0000

a commit
";
        assert_eq!(raw_object.body, expected_body);
    }
}
