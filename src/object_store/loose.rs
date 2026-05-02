use miniz_oxide::inflate::decompress_to_vec_zlib;
use nom::{
    Parser,
    branch::alt,
    bytes::complete::tag,
    character::complete::{char, u64},
    sequence::terminated,
};

use crate::{
    error::{Error, GResult},
    file_system::{Directory, File, FileSystem, FileSystemError, Offset},
    object::ObjectId,
    object_store::{ObjectSize, ObjectType, RawObject},
    repo::Repo,
};

async fn get_loose_object_file<F: FileSystem>(
    repo: &Repo<F>,
    id: ObjectId,
) -> GResult<Option<F::File>> {
    let (prefix, suffix) = id.bytes().split_at(1);
    let mut prefix_buf = [0u8; 2];
    hex::encode_to_slice(prefix, &mut prefix_buf).unwrap();
    let mut suffix_buf = [0u8; 2 * 19];
    hex::encode_to_slice(suffix, &mut suffix_buf).unwrap();
    let mut dir = repo.git_dir.open_subdir(b"objects").await?;
    dir = match dir.open_subdir(&prefix_buf).await {
        Ok(d) => d,
        Err(FileSystemError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let file = match dir.open_file(&suffix_buf).await {
        Ok(f) => f,
        Err(FileSystemError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    Ok(Some(file))
}

pub(crate) async fn read_loose_object_size_type<F: FileSystem>(
    repo: &Repo<F>,
    id: ObjectId,
) -> GResult<Option<(ObjectSize, ObjectType)>> {
    let file = get_loose_object_file(repo, id).await?;
    let Some(mut file) = file else {
        return Ok(None);
    };
    let mut buf = [0u8; 32];
    file.read_segment(Offset(0), &mut buf).await?;
    let (_, (size, object_type)) = parse_header(&buf).map_err(|_| Error::MalformedObject(id))?;
    Ok(Some((size, object_type)))
}

pub(crate) async fn read_loose_object<F: FileSystem>(
    repo: &Repo<F>,
    id: ObjectId,
) -> GResult<Option<RawObject>> {
    let file = get_loose_object_file(repo, id).await?;
    let Some(mut file) = file else {
        return Ok(None);
    };
    let data = file.read_all().await?;
    let data = decompress_to_vec_zlib(&data).map_err(|e| Error::LooseObjectDecompressError {
        id,
        status: e.status,
    })?;
    let (body, (_, object_type)) = parse_header(&data).map_err(|_| Error::MalformedObject(id))?;
    Ok(Some(RawObject {
        object_type,
        body: body.to_vec(),
    }))
}

fn parse_header(input: &[u8]) -> nom::IResult<&[u8], (ObjectSize, ObjectType)> {
    let (rest, (object_type, size)) = (
        terminated(
            alt((
                tag("commit").map(|_| ObjectType::Commit),
                tag("tag").map(|_| ObjectType::Tag),
                tag("tree").map(|_| ObjectType::Tree),
                tag("blob").map(|_| ObjectType::Blob),
            )),
            char(' '),
        ),
        terminated(u64, char('\0')).map(ObjectSize),
    )
        .parse(input)?;
    Ok((rest, (size, object_type)))
}

#[cfg(test)]
mod tests {
    use crate::test::helpers::make_basic_repo;
    use futures::executor::block_on;
    use hex_literal::hex;

    use super::*;

    #[test]
    fn test_read_loose_object_existing() {
        let test_repo = make_basic_repo().unwrap();
        let commit_id = test_repo.run_git(["rev-parse", "HEAD"]).unwrap();
        let commit_id = ObjectId::from_hex(commit_id.trim_ascii()).unwrap();

        let repo = test_repo.repo();
        let object = block_on(read_loose_object(&repo, commit_id))
            .unwrap()
            .unwrap();
        assert_eq!(object.object_type, ObjectType::Commit);
        assert_eq!(
            object.body,
            b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a user <an-email-address> 946684800 +0000
committer a user <an-email-address> 946684800 +0000

a commit
"
        );
    }

    #[test]
    fn test_read_loose_object_nonexistent() {
        let test_repo = make_basic_repo().unwrap();
        let repo = test_repo.repo();
        let object = block_on(read_loose_object(
            &repo,
            ObjectId::from_bytes(hex!("0000000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert!(object.is_none());
    }
}
