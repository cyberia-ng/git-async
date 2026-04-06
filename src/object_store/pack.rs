use crate::{
    directory::File,
    error::{Error, GResult},
    object_store::RawObjectType,
};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::matches;
use miniz_oxide::inflate::{
    DecompressError, TINFLStatus,
    core::{
        DecompressorOxide, decompress,
        inflate_flags::{
            TINFL_FLAG_HAS_MORE_INPUT, TINFL_FLAG_PARSE_ZLIB_HEADER,
            TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
        },
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PackObjectType {
    Base(RawObjectType),
    OffsetDelta {
        base_offset_neg: u64,
    },
    #[allow(dead_code)]
    RefDelta, // TODO
}

#[derive(Debug)]
pub(crate) struct PackObject {
    pub object_type: PackObjectType,
    pub body: Vec<u8>,
}

// Git uses three slightly different algorithms for encoding variable-width
// integers in different contexts within the packfile. This is not documented
// anywhere.

fn read_obj_type_size(buf: &[u8]) -> GResult<(usize, PackObjectType, u64)> {
    // This algorithm is for reading the first part of the packfile object
    // header, which encodes the object type and size.
    let mut pos: usize = 0;
    let mut object_type: Option<PackObjectType> = None;
    let mut obj_size: u64 = 0;
    let mut done_accumulating_size = false;
    for buf_byte in buf.iter() {
        done_accumulating_size = (0b10000000 & *buf_byte) == 0;
        if pos == 0 {
            let obj_type_id = 0b01110000 & *buf_byte;
            object_type = Some(match obj_type_id {
                0b00010000 => PackObjectType::Base(RawObjectType::Commit),
                0b00100000 => PackObjectType::Base(RawObjectType::Tree),
                0b00110000 => PackObjectType::Base(RawObjectType::Blob),
                0b01000000 => PackObjectType::Base(RawObjectType::Tag),
                0b01100000 => PackObjectType::OffsetDelta { base_offset_neg: 0 },
                0b01110000 => unimplemented!(), // TODO
                _ => return Err(Error::MalformedPackObject),
            });
            let size_bits = 0b00001111 & *buf_byte;
            obj_size = size_bits.into();
        } else {
            let size_bits = 0b01111111 & *buf_byte;
            let shift: usize = 4 + 7 * (pos - 1);
            obj_size += (size_bits as u64) << shift;
        }
        pos += 1;
        if done_accumulating_size {
            break;
        }
    }
    if !done_accumulating_size {
        panic!("buffer was too short to hold varsize");
    }
    Ok((pos, object_type.unwrap(), obj_size))
}

fn read_delta_offset_size(buf: &[u8]) -> (usize, u64) {
    // This algorithm is for reading the second part of the packfile object
    // header (in the case of an offset delta object), which encodes the
    // relative negative offset of the delta object's base object
    let mut bytes_read = 0;
    let mut size = 0;
    let mut done_accumulating_size = false;
    for (buf_idx, buf_byte) in buf.iter().enumerate() {
        done_accumulating_size = (0b10000000 & *buf_byte) == 0;
        if buf_idx != 0 {
            size += 1;
        }
        size <<= 7;
        size += u64::from(buf_byte & 0b01111111);
        bytes_read += 1;
        if done_accumulating_size {
            break;
        }
    }
    if !done_accumulating_size {
        panic!("buffer was too short to hold varsize");
    }
    (bytes_read, size)
}

fn read_delta_expected_size(buf: &[u8]) -> (usize, u64) {
    // This algorithm is for reading the expected base object and un-deltified
    // object sizes, which form the header of the decompressed data stream in an
    // offset delta object.
    let mut bytes_read = 0;
    let mut size = 0;
    let mut done_accumulating_size = false;
    let mut shift = 0;
    for buf_byte in buf.iter() {
        done_accumulating_size = (0b10000000 & *buf_byte) == 0;
        size += u64::from(buf_byte & 0b01111111) << shift;
        shift += 7;
        bytes_read += 1;
        if done_accumulating_size {
            break;
        }
    }
    if !done_accumulating_size {
        panic!("buffer was too short to hold varsize");
    }
    (bytes_read, size)
}

async fn read_pack_object<F: File>(pack_file: &mut F, offset: u64) -> GResult<PackObject> {
    let mut buf = [0u8; 4];
    pack_file.read_segment(0, &mut buf).await?;
    if buf != *b"PACK" {
        return Err(Error::UnsupportedPackVersion);
    }
    pack_file.read_segment(4, &mut buf).await?;
    if buf != [0, 0, 0, 2] {
        return Err(Error::UnsupportedPackVersion);
    }

    // buf size must be enough to encode a u64::MAX in git's variable size
    // encoding - i.e. at least 10 bytes
    let mut buf = [0u8; 32];
    let mut pos: usize = 0;
    pack_file
        .read_segment(offset + u64::try_from(pos).unwrap(), &mut buf)
        .await?;
    let (bytes_read, mut object_type, obj_size) = read_obj_type_size(&buf)?;
    pos += bytes_read;
    let obj_size = usize::try_from(obj_size).unwrap();

    if let PackObjectType::OffsetDelta {
        ref mut base_offset_neg,
    } = object_type
    {
        pack_file
            .read_segment(offset + u64::try_from(pos).unwrap(), &mut buf)
            .await?;
        let (bytes_read, size) = read_delta_offset_size(&buf);
        *base_offset_neg = size;
        pos += bytes_read;
    }

    let mut compressed_body_buf = vec![0u8; 4096];
    let mut body = vec![0u8; obj_size.next_power_of_two()];
    let mut state = Box::<DecompressorOxide>::default();
    let mut out_idx: usize = 0;
    loop {
        pack_file
            .read_segment(
                offset + u64::try_from(pos).unwrap(),
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
        use TINFLStatus::*;
        match status {
            Done => break,
            NeedsMoreInput => {}
            _ => {
                return Err(Error::DecompressError(DecompressError {
                    status,
                    output: Vec::new(),
                }));
            }
        }
    }
    body.truncate(obj_size);
    Ok(PackObject { object_type, body })
}

pub(crate) async fn form_deltified_chain<F: File>(
    pack_file: &mut F,
    start_offset: u64,
) -> GResult<(Vec<PackObject>, PackObject)> {
    let mut chain = Vec::new();
    let mut final_pack_object: Option<PackObject> = None;
    let mut offset = start_offset;
    while final_pack_object.is_none() {
        let object = read_pack_object(pack_file, offset).await?;
        match &object.object_type {
            PackObjectType::OffsetDelta { base_offset_neg } => {
                offset -= base_offset_neg;
                chain.push(object);
            }
            _ => {
                final_pack_object = Some(object);
            }
        }
    }
    Ok((chain, final_pack_object.unwrap()))
}

fn reconstruct_deltified_object(deltified: &[u8], base: &[u8]) -> Vec<u8> {
    let mut pos: usize = 0;
    let mut reconstructed_body: Vec<u8> = Vec::new();
    let (bytes_read, base_object_size) = read_delta_expected_size(&deltified[pos..]);
    pos += bytes_read;
    debug_assert_eq!(
        base_object_size,
        base.len().try_into().unwrap(),
        "base size"
    );
    let (bytes_read, reconstructed_body_size) = read_delta_expected_size(&deltified[pos..]);
    pos += bytes_read;
    while pos < deltified.len() {
        let mut instruction = deltified[pos];
        pos += 1;
        if instruction & 0b10000000 == 0 {
            // Append
            let size = usize::from(instruction & 0b01111111);
            reconstructed_body.extend_from_slice(&deltified[pos..(pos + size)]);
            pos += size;
        } else {
            // Copy
            let mut offset = [0u8; 4];
            let mut size = [0u8; 4];
            for offset_byte in offset.iter_mut() {
                if instruction & 1 != 0 {
                    *offset_byte = deltified[pos];
                    pos += 1;
                }
                instruction >>= 1;
            }
            for size_byte in size[..3].iter_mut() {
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
        reconstructed_body.len(),
        reconstructed_body_size.try_into().unwrap(),
        "reconstructed size"
    );
    reconstructed_body
}

pub(crate) fn reconstruct_deltified_object_from_chain(
    chain: &[PackObject],
    final_object: &PackObject,
) -> PackObject {
    // TODO: when we implement ref deltas, this should take another parameter of the real base object

    debug_assert!(
        chain.iter().all(|item| {
            matches!(
                item.object_type,
                PackObjectType::OffsetDelta { base_offset_neg: _ }
            )
        }),
        "chain contains offsets"
    );
    debug_assert!(
        match final_object.object_type {
            PackObjectType::OffsetDelta { base_offset_neg: _ } => false,
            PackObjectType::RefDelta => todo!(),
            _ => true,
        },
        "final object is base"
    );
    let chain_iter = chain.iter().rev();
    let mut reconstructed_body = final_object.body.clone();
    for pack_object in chain_iter {
        reconstructed_body = reconstruct_deltified_object(&pack_object.body, &reconstructed_body);
    }
    PackObject {
        object_type: final_object.object_type,
        body: reconstructed_body,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::helpers::{make_basic_repo, make_packfile_repo, make_similar_commits};
    use futures::executor::block_on;
    use hex_literal::hex;

    use super::*;

    #[test]
    fn read_non_deltified_commit() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let pack_object = block_on(read_pack_object(&mut pack_file, 0x0c)).unwrap();
        assert_eq!(
            pack_object.object_type,
            PackObjectType::Base(RawObjectType::Commit)
        );
        let expected = b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a user <an-email-address> 946684800 +0000
committer a user <an-email-address> 946684800 +0000

a commit
";
        assert_eq!(pack_object.body, expected);
    }

    #[test]
    fn read_non_deltified_blob() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let pack_object = block_on(read_pack_object(&mut pack_file, 0x11e)).unwrap();
        assert_eq!(
            pack_object.object_type,
            PackObjectType::Base(RawObjectType::Blob)
        );
        assert_eq!(pack_object.body, b"");
    }

    #[test]
    fn read_non_deltified_tree() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let pack_object = block_on(read_pack_object(&mut pack_file, 0xf1)).unwrap();
        assert_eq!(
            pack_object.object_type,
            PackObjectType::Base(RawObjectType::Tree)
        );
        let mut expected = Vec::new();
        expected.extend_from_slice(b"100644 a-file\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        assert_eq!(pack_object.body, expected);
    }

    #[test]
    fn read_non_deltified_tag() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let pack_object = block_on(read_pack_object(&mut pack_file, 0x7a)).unwrap();
        assert_eq!(
            pack_object.object_type,
            PackObjectType::Base(RawObjectType::Tag)
        );
        assert_eq!(
            pack_object.body,
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
        let mut pack_file = test_repo
            .pack_file(b"9a21071e8cb13ec8f89907ff140f8e8c20e978c1")
            .unwrap();
        let pack_object = block_on(read_pack_object(&mut pack_file, 786)).unwrap();
        assert_eq!(
            pack_object.object_type,
            PackObjectType::OffsetDelta {
                base_offset_neg: 128
            }
        );
        assert_eq!(
            pack_object.body,
            hex!("94 06 f7 05 b0 85 01 b3 a2 01 72 01")
        );
    }

    #[test]
    fn form_deltified_object_chain() {
        let test_repo = make_basic_repo().unwrap();
        make_similar_commits(&test_repo).unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let mut pack_file = test_repo
            .pack_file(b"9a21071e8cb13ec8f89907ff140f8e8c20e978c1")
            .unwrap();
        let (chain, final_object) = block_on(form_deltified_chain(&mut pack_file, 809)).unwrap();
        assert_eq!(chain.len(), 2);
        for object in chain {
            assert!(matches!(
                object.object_type,
                PackObjectType::OffsetDelta { base_offset_neg: _ }
            ));
        }
        if let PackObjectType::OffsetDelta { base_offset_neg: _ } = final_object.object_type {
            panic!()
        }
    }

    #[test]
    fn reconstruct_one_object() {
        let mut base_object = vec![0u8; 128 * 1024];
        for (i, item) in base_object.iter_mut().enumerate() {
            *item = (i % u8::MAX as usize) as u8;
        }

        let mut deltified_object = Vec::new();
        let base_object_size_encoded: [u8; _] = [0b10000000, 0b10000000, 0b00001000]; // 128 * 1024
        assert_eq!(
            read_delta_expected_size(&base_object_size_encoded).1,
            128 * 1024
        );
        let target_object_size_encoded: [u8; _] = [0b10001101, 0b10000000, 0b00000100]; // 10 + 3 + 0x10000
        assert_eq!(
            read_delta_expected_size(&target_object_size_encoded).1,
            10 + 3 + 0x10000
        );

        deltified_object.extend_from_slice(&base_object_size_encoded);
        deltified_object.extend_from_slice(&target_object_size_encoded);

        // Small copy
        let offset_1: u32 = 65;
        let size_1: u32 = 10;
        let instruction_1: [u8; _] = [0b10010001, 65, 10];
        deltified_object.extend_from_slice(&instruction_1);

        // Append
        let instruction_2: [u8; _] = [0b00000011, 0xc0, 0xff, 0xee];
        deltified_object.extend_from_slice(&instruction_2);

        // Copy with special case size = 0 (interpeted as size = 0b10000)
        let offset_3: u32 = 0x10000;
        let instruction_3: [u8; _] = [0b10000100, 0x01];
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
        let mut pack_file = test_repo
            .pack_file(b"9a21071e8cb13ec8f89907ff140f8e8c20e978c1")
            .unwrap();
        let (chain, final_object) = block_on(form_deltified_chain(&mut pack_file, 809)).unwrap();
        let object = reconstruct_deltified_object_from_chain(&chain, &final_object);
        assert_eq!(
            object.object_type,
            PackObjectType::Base(RawObjectType::Tree)
        );
        let mut expected = Vec::new();
        expected.extend_from_slice(b"100644 a\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        expected.extend_from_slice(b"100644 a-file\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        for c in b'b'..(b'z' + 1) {
            if c != b'm' && c != b't' {
                expected.extend_from_slice(b"100644 ");
                expected.push(c);
                expected.push(b'\0');
                expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
            }
        }
        assert_eq!(object.body, expected);
    }
}
