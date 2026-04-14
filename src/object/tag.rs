use crate::{
    error::{Error, GResult},
    object::{ObjectHeader, ObjectId, parse_author_committer_tagger, parse_object_headers},
    parsing::{ParseError, ParseResult},
    repo::Repo,
};
use accessory::Accessors;
use alloc::vec::Vec;
use chrono::{DateTime, FixedOffset};
use nom::{Parser, combinator::all_consuming};
#[cfg(feature = "serde")]
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum TagType {
    Commit,
    Blob,
    Tree,
    Tag,
}

#[derive(Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Tag<'r, D> {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get(cp))]
    target: ObjectId,

    #[access(get(cp))]
    tag_type: TagType,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    name: Vec<u8>,

    #[access(get(as_ref, ty(Option<&Vec<u8>>)))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::option_utf8"))]
    tagger_name: Option<Vec<u8>>,

    #[access(get(as_ref, ty(Option<&Vec<u8>>)))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::option_utf8"))]
    tagger_email: Option<Vec<u8>>,

    #[access(get(cp))]
    tag_date: Option<DateTime<FixedOffset>>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    message: Vec<u8>,

    #[access(get(ty(&[ObjectHeader])))]
    additional_headers: Vec<ObjectHeader>,

    #[cfg_attr(feature = "serde", serde(skip))]
    repo: Option<&'r Repo<D>>,
}

impl<'r, D> Tag<'r, D> {
    pub(crate) fn parser<'a>(
        id: ObjectId,
        repo: &'r Repo<D>,
    ) -> impl Fn(&'a [u8]) -> ParseResult<&'a [u8], Self> {
        move |input: &[u8]| {
            let (message, raw_headers) = parse_object_headers.parse(input)?;
            let mut object: Option<ObjectId> = None;
            let mut tag_type: Option<TagType> = None;
            let mut tag: Option<Vec<u8>> = None;
            let mut tagger_name: Option<Vec<u8>> = None;
            let mut tagger_email: Option<Vec<u8>> = None;
            let mut tag_date: Option<DateTime<FixedOffset>> = None;
            let mut additional_headers = Vec::new();
            for ObjectHeader { name, value } in raw_headers {
                match name.as_slice() {
                    b"object" => {
                        let (_, object_id) = all_consuming(ObjectId::parse).parse(&value)?;
                        object = Some(object_id)
                    }
                    b"type" => {
                        tag_type = match value.as_slice() {
                            b"commit" => Some(TagType::Commit),
                            b"blob" => Some(TagType::Blob),
                            b"tree" => Some(TagType::Tree),
                            b"tag" => Some(TagType::Tag),
                            _ => None,
                        };
                    }
                    b"tag" => tag = Some(value),
                    b"tagger" => {
                        let (_, (name, email, date)) =
                            all_consuming(parse_author_committer_tagger).parse(&value)?;
                        tagger_name = Some(name.to_vec());
                        tagger_email = Some(email.to_vec());
                        tag_date = Some(date);
                    }
                    _ => {
                        additional_headers.push(ObjectHeader { name, value });
                    }
                }
            }
            let f = move || -> Option<Tag<'r, D>> {
                Some(Tag {
                    id,
                    target: object?,
                    tag_type: tag_type?,
                    name: tag?,
                    tagger_name,
                    tagger_email,
                    tag_date,
                    message: message.to_vec(),
                    additional_headers,
                    repo: Some(repo),
                })
            };
            match f() {
                None => Err(nom::Err::Failure(ParseError::MissingFields)),
                Some(tag) => Ok((&[][..], tag)),
            }
        }
    }

    pub(crate) fn repo(&self) -> GResult<&'r Repo<D>> {
        match self.repo {
            Some(r) => Ok(r),
            None => Err(Error::NotAnnotatedWithRepo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::repo::{TestRepo, TestRepoDirectory};
    use hex_literal::hex;

    const ZERO_OID: ObjectId = ObjectId::new([0; 20]);

    fn dummy_repo() -> Repo<TestRepoDirectory> {
        TestRepo::new().unwrap().repo()
    }

    #[test]
    fn parse_commit_tag() {
        let repo = dummy_repo();
        let data = b"object eedeffb6da16ddc3fb61b2255a8259cacc045691
type commit
tag annotated-tag
tagger a-user <an-email-address> 1774822895 +0100

a message
";
        let (_, tag) = Tag::parser(ZERO_OID, &repo).parse(data).unwrap();
        assert_eq!(
            tag.target,
            ObjectId::new(hex!("eedeffb6da16ddc3fb61b2255a8259cacc045691"),)
        );
        assert_eq!(tag.tag_type, TagType::Commit);
        assert_eq!(tag.name, b"annotated-tag");
        assert_eq!(tag.tagger_name.as_deref(), Some(b"a-user".as_slice()));
        assert_eq!(
            tag.tagger_email.as_deref(),
            Some(b"an-email-address".as_slice())
        );
        assert_eq!(
            tag.tag_date,
            Some(DateTime::parse_from_rfc3339("2026-03-29T23:21:35+01:00").unwrap())
        );
        assert_eq!(&tag.message, b"a message\n");
    }

    #[test]
    fn parse_blob_tag() {
        let repo = dummy_repo();
        let data = b"object e69de29bb2d1d6434b8b29ae775ad8c2e48c5391
type blob
tag blob-tag
tagger a-user <an-email-address> 1774826002 +0100

a blob
";
        let (_, tag) = Tag::parser(ZERO_OID, &repo).parse(data).unwrap();
        assert_eq!(tag.tag_type, TagType::Blob);
    }

    #[test]
    fn parse_tree_tag() {
        let repo = dummy_repo();
        let data = b"object 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
type tree
tag tree-tag
tagger a-user <an-email-address> 1774826187 +0100

a tree
";
        let (_, tag) = Tag::parser(ZERO_OID, &repo).parse(data).unwrap();
        assert_eq!(tag.tag_type, TagType::Tree);
    }

    #[test]
    fn parse_nested_tag() {
        let repo = dummy_repo();
        let data = b"object 1c8bf8368bc9b1fd14227c6c1a0b0f30a1812e70
type tag
tag tag-tag
tagger a-user <an-email-address> 1774826312 +0100

a tag
";
        let (_, tag) = Tag::parser(ZERO_OID, &repo).parse(data).unwrap();
        assert_eq!(tag.tag_type, TagType::Tag);
    }
}
