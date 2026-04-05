use crate::{
    directory::File,
    error::{Error, GResult},
};
use alloc::vec;
use alloc::vec::Vec;
use alloc::boxed::Box;
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

#[derive(Debug, PartialEq, Eq)]
enum PackObjectType {
    Commit,
    Blob,
    Tree,
    Tag,
    OffsetDelta { base_offset_neg: u64 },
    RefDelta,
}

async fn read_pack_object<F: File>(
    pack_file: &mut F,
    offset: u64,
) -> GResult<(PackObjectType, Vec<u8>)> {
    let mut buf = [0u8; 4];
    pack_file.read_segment(0, &mut buf).await?;
    if buf != *b"PACK" {
        return Err(Error::UnsupportedPackVersion);
    }
    pack_file.read_segment(4, &mut buf).await?;
    if buf != [0, 0, 0, 2] {
        return Err(Error::UnsupportedPackVersion);
    }

    let mut buf = [0u8; 32];
    let mut obj_type: Option<PackObjectType> = None;
    let mut obj_size: usize = 0;
    let mut done_accumulating_size = false;
    let mut idx: usize = 0;
    while !done_accumulating_size {
        pack_file
            .read_segment(offset + u64::try_from(idx).unwrap(), &mut buf)
            .await?;
        for buf_byte in buf.iter() {
            done_accumulating_size = (0b10000000 & *buf_byte) == 0;
            if idx == 0 {
                let obj_type_id = 0b01110000 & *buf_byte;
                obj_type = Some(match obj_type_id {
                    0b00010000 => PackObjectType::Commit,
                    0b00100000 => PackObjectType::Tree,
                    0b00110000 => PackObjectType::Blob,
                    0b01000000 => PackObjectType::Tag,
                    0b01100000 => PackObjectType::OffsetDelta { base_offset_neg: 0 },
                    _ => todo!(),
                });
                let size_bits = 0b00001111 & *buf_byte;
                obj_size = size_bits.into();
            } else {
                let size_bits = 0b01111111 & *buf_byte;
                let shift: usize = 4 + 7 * (idx - 1);
                obj_size += (size_bits as usize) << shift;
            }
            idx += 1;
            if done_accumulating_size {
                break;
            }
        }
    }
    let mut obj_type = obj_type.unwrap();

    if let PackObjectType::OffsetDelta {
        ref mut base_offset_neg,
    } = obj_type
    {
        let mut done_accumulating_base_offset = false;
        let mut first = true;
        while !done_accumulating_base_offset {
            pack_file
                .read_segment(offset + u64::try_from(idx).unwrap(), &mut buf)
                .await?;
            for buf_byte in buf.iter() {
                done_accumulating_base_offset = (0b10000000 & *buf_byte) == 0;
                if !first {
                    *base_offset_neg += 1;
                }
                *base_offset_neg <<= 7;
                *base_offset_neg += u64::from(buf_byte & 0b01111111);
                idx += 1;
                if done_accumulating_base_offset {
                    break;
                }
                first = false;
            }
        }
    }

    let mut compressed_body_buf = vec![0u8; 4096];
    let mut body = vec![0u8; obj_size.next_power_of_two()];
    let mut state = Box::new(DecompressorOxide::new());
    let mut out_idx: usize = 0;
    loop {
        pack_file
            .read_segment(offset + u64::try_from(idx).unwrap(), &mut compressed_body_buf)
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
        idx += input_read;
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
    Ok((obj_type, body))
}

#[cfg(test)]
mod tests {
    use crate::test::helpers::{make_basic_repo, make_file, make_packfile_repo};
    use futures::executor::block_on;
    use hex_literal::hex;
    use std::fs::remove_file;

    use super::*;

    #[test]
    fn read_non_deltified_commit() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let (obj_type, data) = block_on(read_pack_object(&mut pack_file, 0x0c)).unwrap();
        assert_eq!(obj_type, PackObjectType::Commit);
        let expected = b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a user <an-email-address> 946684800 +0000
committer a user <an-email-address> 946684800 +0000

a commit
";
        assert_eq!(data, expected);
    }

    #[test]
    fn read_non_deltified_blob() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let (obj_type, data) = block_on(read_pack_object(&mut pack_file, 0x11e)).unwrap();
        assert_eq!(obj_type, PackObjectType::Blob);
        assert_eq!(data, b"");
    }

    #[test]
    fn read_non_deltified_tree() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let (obj_type, data) = block_on(read_pack_object(&mut pack_file, 0xf1)).unwrap();
        assert_eq!(obj_type, PackObjectType::Tree);
        let mut expected = Vec::new();
        expected.extend_from_slice(b"100644 a-file\0");
        expected.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        assert_eq!(data, expected);
    }

    #[test]
    fn read_non_deltified_tag() {
        let test_repo = make_packfile_repo().unwrap();
        let mut pack_file = test_repo
            .pack_file(b"220ae2051dba7a9606c35293e9ff1493ff59869f")
            .unwrap();
        let (obj_type, data) = block_on(read_pack_object(&mut pack_file, 0x7a)).unwrap();
        assert_eq!(obj_type, PackObjectType::Tag);
        assert_eq!(
            data,
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
        for chr in 0x61..(0x61 + 26) {
            make_file(&test_repo, str::from_utf8(&[chr]).unwrap()).unwrap();
        }
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit(
                "commit 2",
                "a user",
                "an-email-address",
                "2000-01-01T00:00:00Z",
            )
            .unwrap();
        remove_file(test_repo.location.path().join("z")).unwrap();
        test_repo.run_git(["add", "--all"]).unwrap();
        test_repo
            .commit(
                "commit 3",
                "a user",
                "an-email-address",
                "2000-01-01T00:00:00Z",
            )
            .unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let mut pack_file = test_repo
            .pack_file(b"a18a07ab36ab97d3aff3593cc177406ebfa0eeee")
            .unwrap();
        let (obj_type, body) = block_on(read_pack_object(&mut pack_file, 646)).unwrap();
        assert_eq!(
            obj_type,
            PackObjectType::OffsetDelta {
                base_offset_neg: 128
            }
        );
        assert_eq!(body, hex!("94 06 f7 05 b0 f7 02"));
    }
}
