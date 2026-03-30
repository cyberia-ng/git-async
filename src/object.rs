use crate::{
    directory::Directory,
    error::{Error, GResult},
    repo::Repo,
};
use alloc::vec::Vec;
use chrono::{DateTime, FixedOffset};
use miniz_oxide::inflate::decompress_to_vec_zlib;
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take, take_till, take_until},
    character::complete::{char, hex_digit0, i32, i64, newline, usize},
    combinator::all_consuming,
    multi::many,
    sequence::{delimited, terminated},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectId(pub [u8; 20]);

impl ObjectId {
    pub(crate) fn parse(input: &[u8]) -> nom::IResult<&[u8], Self> {
        take(40usize)
            .and_then(all_consuming(hex_digit0))
            .map_res(|hex_str| {
                let mut buf = [0u8; 20];
                hex::decode_to_slice(hex_str, &mut buf)?;
                Ok::<ObjectId, hex::FromHexError>(ObjectId(buf))
            })
            .parse(input)
    }

    pub fn from_encoded(s: &[u8]) -> Option<Self> {
        let (_, oid) = all_consuming(Self::parse).parse(s).ok()?;
        Some(oid)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Object {
    Commit(Commit),
    Tag(Tag),
    Tree(Tree),
    Blob(Vec<u8>),
}

impl Object {
    pub async fn lookup<D: Directory>(repo: &Repo<D>, id: ObjectId) -> GResult<Self> {
        let (prefix, suffix) = id.0.split_at(1);
        let mut prefix_buf = [0u8; 2];
        hex::encode_to_slice(prefix, &mut prefix_buf)?;
        let mut suffix_buf = [0u8; 2 * 19];
        hex::encode_to_slice(suffix, &mut suffix_buf)?;
        let mut dir = repo.git_dir.open_subdir(b"objects").await?;
        dir = dir.open_subdir(&prefix_buf).await?;
        let data = dir.read_file(&suffix_buf).await?;
        let data = decompress_to_vec_zlib(&data)?;

        let (_, out) = Object::parser(id)
            .parse(data.as_ref())
            .map_err(|_| Error::MalformedObject(id))?;
        Ok(out)
    }

    fn parser<'a>(id: ObjectId) -> impl Fn(&'a [u8]) -> nom::IResult<&'a [u8], Object> {
        fn parse_header_body(input: &[u8]) -> nom::IResult<&[u8], (&[u8], &[u8])> {
            let (rest, (object_type, expected_len)) = (
                terminated(
                    alt((tag("commit"), tag("tag"), tag("tree"), tag("blob"))),
                    char(' '),
                ),
                terminated(usize, char('\0')),
            )
                .parse(input)?;
            let (rest, body) = take(expected_len).parse(rest)?;
            Ok((rest, (object_type, body)))
        }

        move |input: &[u8]| {
            let (_, (object_type, body)) = all_consuming(parse_header_body).parse(input)?;
            let (_, out) = match object_type {
                b"commit" => all_consuming(Commit::parser(id))
                    .map(Object::Commit)
                    .parse(body)?,
                b"tag" => all_consuming(Tag::parser(id))
                    .map(Object::Tag)
                    .parse(body)?,
                b"tree" => all_consuming(Tree::parser(id))
                    .map(Object::Tree)
                    .parse(body)?,
                b"blob" => (&[][..], Object::Blob(body.to_vec())),
                _ => unreachable!(),
            };
            Ok((&[][..], out))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Commit {
    pub id: ObjectId,
    pub author_name: Vec<u8>,
    pub author_email: Vec<u8>,
    pub author_date: DateTime<FixedOffset>,
    pub committer_name: Vec<u8>,
    pub committer_email: Vec<u8>,
    pub commit_date: DateTime<FixedOffset>,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub message: Vec<u8>,
}

impl Commit {
    fn parser<'a>(id: ObjectId) -> impl Fn(&'a [u8]) -> nom::IResult<&'a [u8], Self> {
        move |input| {
            let mut p = (
                delimited(tag("tree "), ObjectId::parse, newline),
                many(0.., delimited(tag("parent "), ObjectId::parse, newline)),
                delimited(tag("author "), parse_author_committer_tagger, newline),
                delimited(
                    tag("committer "),
                    parse_author_committer_tagger,
                    tag("\n\n"),
                ),
            );
            let (
                message,
                (
                    tree_id,
                    parents,
                    (author_name, author_email, author_date),
                    (committer_name, committer_email, commit_date),
                ),
            ) = p.parse(input)?;
            Ok((
                &[][..],
                Commit {
                    id,
                    author_name: author_name.to_vec(),
                    author_email: author_email.to_vec(),
                    author_date,
                    committer_name: committer_name.to_vec(),
                    committer_email: committer_email.to_vec(),
                    commit_date,
                    tree: tree_id,
                    parents,
                    message: message.to_vec(),
                },
            ))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TagType {
    Commit,
    Blob,
    Tree,
    Tag,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tag {
    pub id: ObjectId,
    pub object: ObjectId,
    pub tag_type: TagType,
    pub tag: Vec<u8>,
    pub tagger_name: Vec<u8>,
    pub tagger_email: Vec<u8>,
    pub tag_date: DateTime<FixedOffset>,
    pub message: Vec<u8>,
}

impl Tag {
    fn parser<'a>(id: ObjectId) -> impl Fn(&'a [u8]) -> nom::IResult<&'a [u8], Self> {
        move |input| {
            let mut p = (
                delimited(tag("object "), ObjectId::parse, newline),
                delimited(
                    tag("type "),
                    alt((
                        tag("commit").map(|_| TagType::Commit),
                        tag("blob").map(|_| TagType::Blob),
                        tag("tree").map(|_| TagType::Tree),
                        tag("tag").map(|_| TagType::Tag),
                    )),
                    newline,
                ),
                delimited(tag("tag "), take_till(|c| c == b'\n'), newline),
                delimited(tag("tagger "), parse_author_committer_tagger, tag("\n\n")),
            );
            let (message, (object, tag_type, tag, (tagger_name, tagger_email, tag_date))) =
                p.parse(input)?;
            Ok((
                &[][..],
                Tag {
                    id,
                    object,
                    tag_type,
                    tag: tag.to_vec(),
                    tagger_name: tagger_name.to_vec(),
                    tagger_email: tagger_email.to_vec(),
                    tag_date,
                    message: message.to_vec(),
                },
            ))
        }
    }
}

#[allow(clippy::type_complexity)]
fn parse_author_committer_tagger(
    input: &[u8],
) -> nom::IResult<&[u8], (&[u8], &[u8], DateTime<FixedOffset>)> {
    (
        terminated(take_until(" <"), tag(" <")),
        terminated(take_until("> "), tag("> ")),
        (
            terminated(i64, char(' ')),
            alt((char('+').map(|_| 1), char('-').map(|_| -1))),
            take(2usize).and_then(all_consuming(i32)),
            take(2usize).and_then(all_consuming(i32)),
        )
            .map_opt(|(timestamp, tz_sign, tz_hour, tz_minute)| {
                let date = DateTime::from_timestamp(timestamp, 0)?;
                let offset = FixedOffset::east_opt(tz_sign * (3600 * tz_hour + 60 * tz_minute))?;
                let author_date = date.with_timezone(&offset);
                Some(author_date)
            }),
    )
        .parse(input)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TreeEntryType {
    File,
    Executable,
    Symlink,
    Tree,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub name: Vec<u8>,
    pub entry_type: TreeEntryType,
    pub id: ObjectId,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tree {
    pub id: ObjectId,
    pub entries: Vec<TreeEntry>,
}

impl TreeEntry {
    fn parser(input: &[u8]) -> nom::IResult<&[u8], Self> {
        let entry_type_parser = alt((
            tag("40000").map(|_| TreeEntryType::Tree),
            tag("100644").map(|_| TreeEntryType::File),
            tag("100755").map(|_| TreeEntryType::Executable),
            tag("120000").map(|_| TreeEntryType::Symlink),
        ));
        let mut p = (
            terminated(entry_type_parser, char(' ')),
            terminated(take_till(|c| c == b'\0'), char('\0')),
            take(20usize).map(|bytes| ObjectId(<[u8; 20]>::try_from(bytes).unwrap())),
        );
        let (rest, (entry_type, name, id)) = p.parse(input)?;
        Ok((
            rest,
            TreeEntry {
                entry_type,
                name: name.to_vec(),
                id,
            },
        ))
    }
}

impl Tree {
    fn parser<'a>(id: ObjectId) -> impl Fn(&'a [u8]) -> nom::IResult<&'a [u8], Self> {
        move |input| {
            many(0.., TreeEntry::parser)
                .map(|entries| Tree { id, entries })
                .parse(input)
        }
    }
}

#[cfg(test)]
mod test {
    use core::iter::zip;

    use super::*;
    use crate::test::repo::{TestRepo, make_basic_commit};
    use futures::executor::block_on;
    use hex_literal::hex;

    #[test]
    fn lookup_commit() {
        let test_repo = TestRepo::new().unwrap();
        make_basic_commit(&test_repo);
        let commit_id = test_repo.run_git(["rev-parse", "HEAD"]).unwrap();
        let commit_id = ObjectId::from_encoded(commit_id.trim_ascii()).unwrap();

        let repo = test_repo.repo();
        let object = block_on(Object::lookup(&repo, commit_id)).unwrap();
        match object {
            Object::Commit(commit) => {
                assert_eq!(commit.id, commit_id);
            }
            Object::Tag(_) => panic!(),
            Object::Tree(_) => panic!(),
            Object::Blob(_) => panic!(),
        }
    }

    #[test]
    fn test_parse_object_invalid_length() {
        let data = b"commit 169\0tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a-user <an-email-address> 1774735018 +0530
committer another-user <another-email-address> 1774735019 -0800

a commit
";
        let result = Object::parser(ObjectId([0u8; 20])).parse(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_root_commit() {
        let data = b"commit 170\0tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a-user <an-email-address> 1774735018 +0530
committer another-user <another-email-address> 1774735019 -0800

a commit
";
        let (rest, object) = Object::parser(ObjectId([0u8; 20])).parse(data).unwrap();
        assert_eq!(rest, &[]);
        let commit = match object {
            Object::Commit(commit) => commit,
            _ => panic!(),
        };
        assert_eq!(&commit.parents, &[]);
        assert_eq!(
            commit.tree,
            ObjectId(hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb"),)
        );
        assert_eq!(str::from_utf8(&commit.author_name).unwrap(), "a-user");
        assert_eq!(
            str::from_utf8(&commit.author_email).unwrap(),
            "an-email-address"
        );
        assert_eq!(
            commit.author_date,
            DateTime::parse_from_rfc3339("2026-03-29T03:26:58+05:30").unwrap()
        );
        assert_eq!(
            str::from_utf8(&commit.committer_name).unwrap(),
            "another-user"
        );
        assert_eq!(
            str::from_utf8(&commit.committer_email).unwrap(),
            "another-email-address"
        );
        assert_eq!(
            commit.commit_date,
            DateTime::parse_from_rfc3339("2026-03-28T13:56:59-08:00").unwrap()
        );
        assert_eq!(str::from_utf8(&commit.message).unwrap(), "a commit\n");
    }

    #[test]
    fn test_parse_normal_commit() {
        let data = b"commit 213\0tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904
parent 16dafd3d0ba5af72f035d641c076a4150eda548d
author a-user <an-email-address> 1774739676 +0000
committer a-user <an-email-address> 1774739676 +0000

another commit
";
        let (_, object) = Object::parser(ObjectId([0u8; 20])).parse(data).unwrap();
        let commit = match object {
            Object::Commit(commit) => commit,
            _ => panic!(),
        };
        assert_eq!(
            &commit.parents,
            &[ObjectId(hex!("16dafd3d0ba5af72f035d641c076a4150eda548d"),)]
        );
    }

    #[test]
    fn test_parse_merge_commit() {
        let data = b"commit 268\0tree bfb6d701e108f3be27395bd60c3417b47ffbe7d9
parent f625376d12f2edc71cff70bb42d387ddf2408460
parent 6904799d30a34bfcf6ca6a3526fc8b771ed6705c
author a-user <an-email-address> 1774740069 +0000
committer a-user <an-email-address> 1774740069 +0000

Merge branch 'branch'
";
        let (_, object) = Object::parser(ObjectId([0u8; 20])).parse(data).unwrap();
        let commit = match object {
            Object::Commit(commit) => commit,
            _ => panic!(),
        };
        assert_eq!(commit.parents.len(), 2);
    }

    #[test]
    fn test_parse_author_committer_line() {
        let example = "an author <an-email-address> 0 +0000";
        parse_author_committer_tagger(example.as_bytes()).unwrap();
    }

    #[test]
    fn parse_commit_tag() {
        let data = b"tag 139\0object eedeffb6da16ddc3fb61b2255a8259cacc045691
type commit
tag annotated-tag
tagger a-user <an-email-address> 1774822895 +0100

a message
";
        let (_, object) = Object::parser(ObjectId([0u8; 20])).parse(data).unwrap();
        let tag = match object {
            Object::Tag(tag) => tag,
            _ => panic!(),
        };
        assert_eq!(
            tag.object,
            ObjectId(hex!("eedeffb6da16ddc3fb61b2255a8259cacc045691"),)
        );
        assert_eq!(tag.tag_type, TagType::Commit);
        assert_eq!(tag.tag, b"annotated-tag");
        assert_eq!(tag.tagger_name, b"a-user");
        assert_eq!(tag.tagger_email, b"an-email-address");
        assert_eq!(
            tag.tag_date,
            DateTime::parse_from_rfc3339("2026-03-29T23:21:35+01:00").unwrap()
        );
        assert_eq!(&tag.message, b"a message\n");
    }

    #[test]
    fn parse_blob_tag() {
        let data = b"tag 129\0object e69de29bb2d1d6434b8b29ae775ad8c2e48c5391
type blob
tag blob-tag
tagger a-user <an-email-address> 1774826002 +0100

a blob
";
        let (_, object) = Object::parser(ObjectId([0u8; 20])).parse(data).unwrap();
        let tag = match object {
            Object::Tag(tag) => tag,
            _ => panic!(),
        };
        assert_eq!(tag.tag_type, TagType::Blob);
    }

    #[test]
    fn parse_tree_tag() {
        let data = b"tag 129\0object 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
type tree
tag tree-tag
tagger a-user <an-email-address> 1774826187 +0100

a tree
";
        let (_, object) = Object::parser(ObjectId([0u8; 20])).parse(data).unwrap();
        let tag = match object {
            Object::Tag(tag) => tag,
            _ => panic!(),
        };
        assert_eq!(tag.tag_type, TagType::Tree);
    }

    #[test]
    fn parse_nested_tag() {
        let data = b"tag 126\0object 1c8bf8368bc9b1fd14227c6c1a0b0f30a1812e70
type tag
tag tag-tag
tagger a-user <an-email-address> 1774826312 +0100

a tag
";
        let (_, object) = Object::parser(ObjectId([0u8; 20])).parse(data).unwrap();
        let tag = match object {
            Object::Tag(tag) => tag,
            _ => panic!(),
        };
        assert_eq!(tag.tag_type, TagType::Tag);
    }

    #[test]
    fn parse_tree() {
        let mut data = Vec::new();
        data.extend_from_slice(b"tree 155\0");
        data.extend_from_slice(b"40000 a-directory\0");
        data.extend_from_slice(&hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb"));
        data.extend_from_slice(b"100644 a-file\0");
        data.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        data.extend_from_slice(b"120000 a-symlink\0");
        data.extend_from_slice(&hex!("7c35e066a9001b24677ae572214d292cebc55979"));
        data.extend_from_slice(b"100755 an-executable-file\0");
        data.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        let (_, object) = Object::parser(ObjectId([0u8; 20])).parse(&data).unwrap();
        let tree = match object {
            Object::Tree(tree) => tree,
            _ => panic!(),
        };
        let expected_entries = [
            TreeEntry {
                entry_type: TreeEntryType::Tree,
                id: ObjectId(hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb")),
                name: Vec::from(b"a-directory"),
            },
            TreeEntry {
                entry_type: TreeEntryType::File,
                id: ObjectId(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                name: Vec::from(b"a-file"),
            },
            TreeEntry {
                entry_type: TreeEntryType::Symlink,
                id: ObjectId(hex!("7c35e066a9001b24677ae572214d292cebc55979")),
                name: Vec::from(b"a-symlink"),
            },
            TreeEntry {
                entry_type: TreeEntryType::Executable,
                id: ObjectId(hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")),
                name: Vec::from(b"an-executable-file"),
            },
        ];
        for (entry, expected) in zip(tree.entries.iter(), expected_entries.iter()) {
            assert_eq!(entry, expected);
        }
    }

    #[test]
    fn parse_empty_blob() {
        let input = b"blob 0\0";
        let (_, object) = Object::parser(ObjectId([0; 20])).parse(input).unwrap();
        let object = match object {
            Object::Blob(blob) => blob,
            _ => panic!(),
        };
        assert_eq!(object, &[]);
    }

    #[test]
    fn parse_contentful_blob() {
        let input = b"blob 11\0hello world";
        let (_, object) = Object::parser(ObjectId([0; 20])).parse(input).unwrap();
        let object = match object {
            Object::Blob(blob) => blob,
            _ => panic!(),
        };
        assert_eq!(object, b"hello world");
    }
}
