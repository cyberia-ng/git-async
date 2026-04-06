use crate::{
    directory::Directory,
    error::{Error, GResult},
    object_store::{RawObject, RawObjectType, lookup::lookup},
    parsing::{ParseError, ParseResult},
    repo::Repo,
};
use accessory::Accessors;
use alloc::format;
use alloc::vec::Vec;
use chrono::{DateTime, FixedOffset};
use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take, take_till, take_until},
    character::complete::{char, hex_digit0, i32, i64, newline, not_line_ending, space1},
    combinator::{all_consuming, not, peek},
    multi::{many, many0},
    sequence::{delimited, preceded, terminated},
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ObjectId(#[cfg_attr(feature = "serde", serde(with = "serde_bytes"))] pub [u8; 20]);

impl alloc::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut chars = [0u8; 40];
        hex::encode_to_slice(self.0, &mut chars).unwrap();
        write!(f, "{}", str::from_utf8(&chars).unwrap())
    }
}

impl alloc::fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ObjectId")
            .field(&format!("{}", self))
            .finish()
    }
}

impl ObjectId {
    pub(crate) fn parse(input: &[u8]) -> ParseResult<&[u8], Self> {
        take(40usize)
            .and_then(all_consuming(hex_digit0))
            .map_res(|hex_str| {
                let mut buf = [0u8; 20];
                hex::decode_to_slice(hex_str, &mut buf)?;
                Ok::<ObjectId, hex::FromHexError>(ObjectId(buf))
            })
            .parse(input)
    }

    pub fn from_hex(s: &[u8]) -> Option<Self> {
        let (_, oid) = all_consuming(Self::parse).parse(s).ok()?;
        Some(oid)
    }
}

#[derive(Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Object<'r, D> {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get)]
    body: ObjectBody,

    #[cfg_attr(feature = "serde", serde(skip))]
    repo: &'r Repo<D>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", content = "value"))]
pub enum ObjectBody {
    Commit(CommitFields),
    Tag(TagFields),
    Tree(TreeFields),
    Blob(Vec<u8>),
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PeeledCommit {
    pub id: ObjectId,
    pub fields: CommitFields,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PeeledTree {
    pub id: ObjectId,
    pub fields: TreeFields,
}

impl<'r, D: Directory> Object<'r, D> {
    pub(crate) async fn lookup(repo: &'r Repo<D>, id: ObjectId) -> GResult<Self> {
        let RawObject {
            object_type,
            body,
            id,
        } = lookup(repo, id)
            .await?
            .ok_or_else(|| Error::MissingObject(id))?;

        let (_, body) = ObjectBody::parser(object_type)
            .parse(body.as_ref())
            .map_err(|_| Error::MalformedObject(id))?;
        Ok(Self { id, body, repo })
    }

    pub async fn peel_to_commit(self) -> GResult<Option<PeeledCommit>> {
        match self.body {
            ObjectBody::Commit(c) => Ok(Some(PeeledCommit {
                id: self.id,
                fields: c,
            })),
            ObjectBody::Tag(TagFields {
                object: object_id, ..
            }) => {
                let object = Object::lookup(self.repo, object_id).await?;
                object.peel_to_commit().await
            }
            _ => Ok(None),
        }
    }

    pub async fn peel_to_tree(self) -> GResult<Option<PeeledTree>> {
        match self.body {
            ObjectBody::Tree(t) => Ok(Some(PeeledTree {
                id: self.id,
                fields: t,
            })),
            ObjectBody::Commit(CommitFields {
                tree: object_id, ..
            }) => {
                let object = Object::lookup(self.repo, object_id).await?;
                object.peel_to_tree().await
            }
            _ => Ok(None),
        }
    }
}

impl ObjectBody {
    fn parser<'a>(object_type: RawObjectType) -> impl Fn(&'a [u8]) -> ParseResult<&'a [u8], Self> {
        move |body: &[u8]| {
            let (_, body) = match object_type {
                RawObjectType::Commit => all_consuming(CommitFields::parser)
                    .map(ObjectBody::Commit)
                    .parse(body)?,
                RawObjectType::Tag => all_consuming(TagFields::parser)
                    .map(ObjectBody::Tag)
                    .parse(body)?,
                RawObjectType::Tree => all_consuming(TreeFields::parser)
                    .map(ObjectBody::Tree)
                    .parse(body)?,
                RawObjectType::Blob => (&[][..], ObjectBody::Blob(body.to_vec())),
            };
            Ok((&[][..], body))
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ObjectHeader {
    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    name: Vec<u8>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    value: Vec<u8>,
}

fn parse_object_headers(input: &[u8]) -> ParseResult<&[u8], Vec<ObjectHeader>> {
    let header = (
        delimited(peek(not(newline)), take_till(|c| c == b' '), char(' ')),
        terminated(
            (
                not_line_ending,
                many0(preceded((newline, space1), not_line_ending)),
            ),
            newline,
        ),
    );
    let mut p = terminated(many0(header), newline);
    let (rest, raw_headers) = p.parse(input)?;
    let mut headers: Vec<ObjectHeader> = Vec::new();
    for (name, (first_line, continuation_lines)) in raw_headers {
        let mut full_line = first_line.to_vec();
        for line in continuation_lines {
            full_line.push(b' ');
            full_line.extend_from_slice(line);
        }
        headers.push(ObjectHeader {
            name: name.to_vec(),
            value: full_line,
        });
    }
    Ok((rest, headers))
}

#[derive(Debug, PartialEq, Eq, Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CommitFields {
    #[access(get(cp))]
    tree: ObjectId,

    #[access(get(ty(&[ObjectId])))]
    parents: Vec<ObjectId>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    author_name: Vec<u8>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    author_email: Vec<u8>,

    #[access(get(cp))]
    author_date: DateTime<FixedOffset>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    committer_name: Vec<u8>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    committer_email: Vec<u8>,

    #[access(get(cp))]
    commit_date: DateTime<FixedOffset>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    message: Vec<u8>,

    #[access(get(ty(&[ObjectHeader])))]
    additional_headers: Vec<ObjectHeader>,
}

impl CommitFields {
    fn parser<'a>(input: &[u8]) -> ParseResult<&'a [u8], Self> {
        let (message, raw_headers) = parse_object_headers.parse(input)?;
        let mut tree_id: Option<ObjectId> = None;
        let mut parents: Vec<ObjectId> = Vec::new();
        let mut author_name: Option<Vec<u8>> = None;
        let mut author_email: Option<Vec<u8>> = None;
        let mut author_date: Option<DateTime<FixedOffset>> = None;
        let mut committer_name: Option<Vec<u8>> = None;
        let mut committer_email: Option<Vec<u8>> = None;
        let mut commit_date: Option<DateTime<FixedOffset>> = None;
        let mut additional_headers: Vec<ObjectHeader> = Vec::new();
        for ObjectHeader { name, value } in raw_headers {
            match name.as_slice() {
                b"tree" => {
                    let (_, object_id) = all_consuming(ObjectId::parse).parse(&value)?;
                    tree_id = Some(object_id);
                }
                b"parent" => {
                    let (_, object_id) = all_consuming(ObjectId::parse).parse(&value)?;
                    parents.push(object_id);
                }
                b"author" => {
                    let (_, (name, email, date)) =
                        all_consuming(parse_author_committer_tagger).parse(&value)?;
                    author_name = Some(name.to_vec());
                    author_email = Some(email.to_vec());
                    author_date = Some(date);
                }
                b"committer" => {
                    let (_, (name, email, date)) =
                        all_consuming(parse_author_committer_tagger).parse(&value)?;
                    committer_name = Some(name.to_vec());
                    committer_email = Some(email.to_vec());
                    commit_date = Some(date);
                }
                _ => {
                    additional_headers.push(ObjectHeader { name, value });
                }
            }
        }
        let f = move || -> Option<CommitFields> {
            Some(CommitFields {
                author_name: Option::map(author_name, Vec::from)?,
                author_email: Option::map(author_email, Vec::from)?,
                author_date: author_date?,
                committer_name: Option::map(committer_name, Vec::from)?,
                committer_email: Option::map(committer_email, Vec::from)?,
                commit_date: commit_date?,
                tree: tree_id?,
                parents,
                message: message.to_vec(),
                additional_headers,
            })
        };
        match f() {
            None => Err(nom::Err::Failure(ParseError::MissingFields)),
            Some(commit) => Ok((&[][..], commit)),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TagType {
    Commit,
    Blob,
    Tree,
    Tag,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TagFields {
    pub object: ObjectId,
    pub tag_type: TagType,
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    pub tag: Vec<u8>,
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    pub tagger_name: Vec<u8>,
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    pub tagger_email: Vec<u8>,
    pub tag_date: DateTime<FixedOffset>,
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    pub message: Vec<u8>,
}

impl TagFields {
    fn parser(input: &[u8]) -> ParseResult<&[u8], Self> {
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
            TagFields {
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

#[allow(clippy::type_complexity)]
fn parse_author_committer_tagger(
    input: &[u8],
) -> ParseResult<&[u8], (&[u8], &[u8], DateTime<FixedOffset>)> {
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

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TreeEntryType {
    File,
    Executable,
    Symlink,
    Tree,
    Commit,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeEntry {
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    pub name: Vec<u8>,
    pub entry_type: TreeEntryType,
    pub id: ObjectId,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeFields {
    pub entries: Vec<TreeEntry>,
}

impl TreeEntry {
    fn parser(input: &[u8]) -> ParseResult<&[u8], Self> {
        let entry_type_parser = alt((
            tag("40000").map(|_| TreeEntryType::Tree),
            tag("100644").map(|_| TreeEntryType::File),
            tag("100755").map(|_| TreeEntryType::Executable),
            tag("120000").map(|_| TreeEntryType::Symlink),
            tag("160000").map(|_| TreeEntryType::Commit),
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

impl TreeFields {
    fn parser(input: &[u8]) -> ParseResult<&[u8], Self> {
        many(0.., TreeEntry::parser)
            .map(|entries| TreeFields { entries })
            .parse(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test::helpers::{make_basic_repo, make_similar_commits};
    use core::iter::zip;
    use futures::executor::block_on;
    use hex_literal::hex;

    #[test]
    fn lookup_commit() {
        let test_repo = make_basic_repo().unwrap();
        let commit_id = test_repo.run_git(["rev-parse", "HEAD"]).unwrap();
        let commit_id = ObjectId::from_hex(commit_id.trim_ascii()).unwrap();

        let repo = test_repo.repo();
        let object = block_on(Object::lookup(&repo, commit_id)).unwrap();
        assert_eq!(object.id, commit_id);
        assert!(matches!(object.body, ObjectBody::Commit(_)));
    }

    #[test]
    fn lookup_packfile_object() {
        let test_repo = make_basic_repo().unwrap();
        make_similar_commits(&test_repo).unwrap();
        test_repo.run_git(["gc"]).unwrap();
        let repo = test_repo.repo();
        let head = block_on(repo.head()).unwrap();
        let commit = match block_on(head.peel_to_object()).unwrap().body {
            ObjectBody::Commit(commit) => commit,
            _ => panic!(),
        };
        let tree_id = commit.tree;
        let tree = match block_on(repo.lookup_object(tree_id)).unwrap().body {
            ObjectBody::Tree(tree) => tree,
            _ => panic!(),
        };
        assert_eq!(tree.entries.len(), 1 + 26 - 2);
    }

    #[test]
    fn parse_root_commit() {
        let data = b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a-user <an-email-address> 1774735018 +0530
committer another-user <another-email-address> 1774735019 -0800

a commit
";
        let (rest, body) = ObjectBody::parser(RawObjectType::Commit)
            .parse(data)
            .unwrap();
        assert_eq!(rest, &[]);
        let commit = match body {
            ObjectBody::Commit(commit) => commit,
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
    fn parse_normal_commit() {
        let data = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904
parent 16dafd3d0ba5af72f035d641c076a4150eda548d
author a-user <an-email-address> 1774739676 +0000
committer a-user <an-email-address> 1774739676 +0000

another commit
";
        let (_, body) = ObjectBody::parser(RawObjectType::Commit)
            .parse(data)
            .unwrap();
        let commit = match body {
            ObjectBody::Commit(commit) => commit,
            _ => panic!(),
        };
        assert_eq!(
            &commit.parents,
            &[ObjectId(hex!("16dafd3d0ba5af72f035d641c076a4150eda548d"),)]
        );
    }

    #[test]
    fn parse_merge_commit() {
        let data = b"tree bfb6d701e108f3be27395bd60c3417b47ffbe7d9
parent f625376d12f2edc71cff70bb42d387ddf2408460
parent 6904799d30a34bfcf6ca6a3526fc8b771ed6705c
author a-user <an-email-address> 1774740069 +0000
committer a-user <an-email-address> 1774740069 +0000

Merge branch 'branch'
";
        let (_, body) = ObjectBody::parser(RawObjectType::Commit)
            .parse(data)
            .unwrap();
        let commit = match body {
            ObjectBody::Commit(commit) => commit,
            _ => panic!(),
        };
        assert_eq!(commit.parents.len(), 2);
    }

    #[test]
    fn parse_author_committer_line() {
        let example = "an author <an-email-address> 0 +0000";
        parse_author_committer_tagger(example.as_bytes()).unwrap();
    }

    #[test]
    fn parse_commit_tag() {
        let data = b"object eedeffb6da16ddc3fb61b2255a8259cacc045691
type commit
tag annotated-tag
tagger a-user <an-email-address> 1774822895 +0100

a message
";
        let (_, body) = ObjectBody::parser(RawObjectType::Tag).parse(data).unwrap();
        let tag = match body {
            ObjectBody::Tag(tag) => tag,
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
        let data = b"object e69de29bb2d1d6434b8b29ae775ad8c2e48c5391
type blob
tag blob-tag
tagger a-user <an-email-address> 1774826002 +0100

a blob
";
        let (_, fields) = ObjectBody::parser(RawObjectType::Tag).parse(data).unwrap();
        let tag = match fields {
            ObjectBody::Tag(tag) => tag,
            _ => panic!(),
        };
        assert_eq!(tag.tag_type, TagType::Blob);
    }

    #[test]
    fn parse_tree_tag() {
        let data = b"object 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
type tree
tag tree-tag
tagger a-user <an-email-address> 1774826187 +0100

a tree
";
        let (_, fields) = ObjectBody::parser(RawObjectType::Tag).parse(data).unwrap();
        let tag = match fields {
            ObjectBody::Tag(tag) => tag,
            _ => panic!(),
        };
        assert_eq!(tag.tag_type, TagType::Tree);
    }

    #[test]
    fn parse_nested_tag() {
        let data = b"object 1c8bf8368bc9b1fd14227c6c1a0b0f30a1812e70
type tag
tag tag-tag
tagger a-user <an-email-address> 1774826312 +0100

a tag
";
        let (_, fields) = ObjectBody::parser(RawObjectType::Tag).parse(data).unwrap();
        let tag = match fields {
            ObjectBody::Tag(tag) => tag,
            _ => panic!(),
        };
        assert_eq!(tag.tag_type, TagType::Tag);
    }

    #[test]
    fn parse_tree() {
        let mut data = Vec::new();
        data.extend_from_slice(b"40000 a-directory\0");
        data.extend_from_slice(&hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb"));
        data.extend_from_slice(b"100644 a-file\0");
        data.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        data.extend_from_slice(b"120000 a-symlink\0");
        data.extend_from_slice(&hex!("7c35e066a9001b24677ae572214d292cebc55979"));
        data.extend_from_slice(b"100755 an-executable-file\0");
        data.extend_from_slice(&hex!("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        data.extend_from_slice(b"160000 a-commit\0");
        data.extend_from_slice(&hex!("91ca81cfccb6f88a34807e9810bb0be409f32d70"));
        let (_, fields) = ObjectBody::parser(RawObjectType::Tree)
            .parse(&data)
            .unwrap();
        let tree = match fields {
            ObjectBody::Tree(tree) => tree,
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
            TreeEntry {
                entry_type: TreeEntryType::Commit,
                id: ObjectId(hex!("91ca81cfccb6f88a34807e9810bb0be409f32d70")),
                name: Vec::from(b"a-commit"),
            },
        ];
        for (entry, expected) in zip(tree.entries.iter(), expected_entries.iter()) {
            assert_eq!(entry, expected);
        }
    }

    #[test]
    fn parse_empty_blob() {
        let input = b"";
        let (_, fields) = ObjectBody::parser(RawObjectType::Blob)
            .parse(input)
            .unwrap();
        let blob = match fields {
            ObjectBody::Blob(blob) => blob,
            _ => panic!(),
        };
        assert_eq!(blob, &[]);
    }

    #[test]
    fn parse_contentful_blob() {
        let input = b"hello world";
        let (_, fields) = ObjectBody::parser(RawObjectType::Blob)
            .parse(input)
            .unwrap();
        let blob = match fields {
            ObjectBody::Blob(blob) => blob,
            _ => panic!(),
        };
        assert_eq!(blob, b"hello world");
    }

    #[test]
    fn parse_commit_additional_headers() {
        let data = b"tree bfb6d701e108f3be27395bd60c3417b47ffbe7d9
parent f625376d12f2edc71cff70bb42d387ddf2408460
author a-user <an-email-address> 1774740069 +0000
committer a-user <an-email-address> 1774740069 +0000
some-header a value
some-other-header a long line-wrapped
  value

the commit message
";
        let (_, fields) = ObjectBody::parser(RawObjectType::Commit)
            .parse(data)
            .unwrap();
        let commit = match fields {
            ObjectBody::Commit(commit) => commit,
            _ => panic!(),
        };
        assert_eq!(commit.additional_headers.len(), 2);
        assert_eq!(
            commit.additional_headers,
            [
                ObjectHeader {
                    name: b"some-header".to_vec(),
                    value: b"a value".to_vec()
                },
                ObjectHeader {
                    name: b"some-other-header".to_vec(),
                    value: b"a long line-wrapped value".to_vec()
                },
            ]
        )
    }
}
