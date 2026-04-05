use miniz_oxide::inflate::decompress_to_vec_zlib;
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take},
    character::complete::{char, usize},
    combinator::all_consuming,
    sequence::terminated,
};

use crate::{
    directory::{Directory, DirectoryError, File},
    error::{Error, GResult},
    object::ObjectId,
    object_store::{RawObject, RawObjectType},
    repo::Repo,
};

pub(crate) async fn read_loose_object<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<RawObject>> {
    let (prefix, suffix) = id.0.split_at(1);
    let mut prefix_buf = [0u8; 2];
    hex::encode_to_slice(prefix, &mut prefix_buf)?;
    let mut suffix_buf = [0u8; 2 * 19];
    hex::encode_to_slice(suffix, &mut suffix_buf)?;
    let mut dir = repo.git_dir.open_subdir(b"objects").await?;
    dir = dir.open_subdir(&prefix_buf).await?;
    let mut file = match dir.open_file(&suffix_buf).await {
        Ok(f) => f,
        Err(DirectoryError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let data = file.read_all().await?;
    let data = decompress_to_vec_zlib(&data)?;
    let (_, (object_type, body)) =
        parse_header_body(&data).map_err(|_| Error::MalformedObject(id))?;
    Ok(Some(RawObject {
        object_type,
        id,
        body: body.to_vec(),
    }))
}

fn parse_header_body(input: &[u8]) -> nom::IResult<&[u8], (RawObjectType, &[u8])> {
    let (rest, (object_type, expected_len)) = (
        terminated(
            alt((
                tag("commit").map(|_| RawObjectType::Commit),
                tag("tag").map(|_| RawObjectType::Tag),
                tag("tree").map(|_| RawObjectType::Tree),
                tag("blob").map(|_| RawObjectType::Blob),
            )),
            char(' '),
        ),
        terminated(usize, char('\0')),
    )
        .parse(input)?;
    let (_, body) = all_consuming(take(expected_len)).parse(rest)?;
    Ok((&[][..], (object_type, body)))
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
        let commit_id = ObjectId::from_encoded(commit_id.trim_ascii()).unwrap();

        let repo = test_repo.repo();
        let object = block_on(read_loose_object(&repo, commit_id))
            .unwrap()
            .unwrap();
        assert_eq!(object.object_type, RawObjectType::Commit);
        assert_eq!(object.id, commit_id);
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
            ObjectId(hex!("0000000000000000000000000000000000000000")),
        ))
        .unwrap();
        assert!(object.is_none());
    }

    #[test]
    fn test_parse_object_invalid_length() {
        let data = b"commit 169\0tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a-user <an-email-address> 1774735018 +0530
committer another-user <another-email-address> 1774735019 -0800

a commit
";
        let result = parse_header_body(data.as_slice());
        assert!(result.is_err());
    }
}
