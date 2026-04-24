use crate::{
    error::GResult,
    object::{
        Object, ObjectId,
        header::{ObjectHeaderIter, RangeObjectHeader},
        parse_author_committer_tagger, range_get, range_get_option,
    },
    parsing::ParseError,
    repo::Repo,
    subslice_range::SubsliceRange,
    traits::AllGenerics,
};
use accessory::Accessors;
use alloc::vec::Vec;
use chrono::{DateTime, FixedOffset};
use core::ops::Range;
use nom::{Parser, combinator::all_consuming};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum TagType {
    Commit,
    Blob,
    Tree,
    Tag,
}

#[derive(Accessors, Clone)]
pub struct Tag {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get(ty(&[u8])))]
    body: Vec<u8>,

    #[access(get(cp))]
    target: ObjectId,

    #[access(get(cp))]
    tag_type: TagType,

    name: Range<usize>,
    tagger_name: Option<Range<usize>>,
    tagger_email: Option<Range<usize>>,
    message: Range<usize>,

    #[access(get(cp))]
    tag_date: Option<DateTime<FixedOffset>>,

    additional_headers: Vec<RangeObjectHeader>,
}

impl PartialEq for Tag {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Tag {}
impl PartialOrd for Tag {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Tag {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl Tag {
    pub async fn lookup_target<G: AllGenerics>(&self, repo: &Repo<G>) -> GResult<Object> {
        repo.lookup_object(self.target).await
    }

    range_get!(name, body);
    range_get_option!(tagger_name, body);
    range_get_option!(tagger_email, body);
    range_get!(message, body);

    pub fn additional_headers(&self) -> ObjectHeaderIter<'_> {
        ObjectHeaderIter::new(self.body.as_slice(), self.additional_headers.as_slice())
    }

    pub(crate) fn parse(id: ObjectId, body: Vec<u8>) -> Result<Self, ParseError> {
        fn f<T>(val: Option<T>) -> Result<T, ParseError> {
            val.ok_or(ParseError::MissingFields)
        }
        let (message, raw_headers) = RangeObjectHeader::parser(&body)?;
        let mut object: Option<ObjectId> = None;
        let mut tag_type: Option<TagType> = None;
        let mut tag: Option<&[u8]> = None;
        let mut tagger_name: Option<&[u8]> = None;
        let mut tagger_email: Option<&[u8]> = None;
        let mut tag_date: Option<DateTime<FixedOffset>> = None;
        let mut additional_headers = Vec::new();
        for (range_header, header) in raw_headers
            .iter()
            .zip(ObjectHeaderIter::new(&body, raw_headers.as_slice()))
        {
            match header.name() {
                b"object" => {
                    let (_, object_id) = all_consuming(ObjectId::parse).parse(header.value())?;
                    object = Some(object_id);
                }
                b"type" => {
                    tag_type = match header.value() {
                        b"commit" => Some(TagType::Commit),
                        b"blob" => Some(TagType::Blob),
                        b"tree" => Some(TagType::Tree),
                        b"tag" => Some(TagType::Tag),
                        _ => None,
                    };
                }
                b"tag" => tag = Some(header.value()),
                b"tagger" => {
                    let (_, (name, email, date)) =
                        all_consuming(parse_author_committer_tagger).parse(header.value())?;
                    tagger_name = Some(name);
                    tagger_email = Some(email);
                    tag_date = Some(date);
                }
                _ => {
                    additional_headers.push(range_header.clone());
                }
            }
        }
        Ok(Tag {
            id,
            target: f(object)?,
            tag_type: f(tag_type)?,
            name: body.subslice_range_stable(f(tag)?).unwrap(),
            tagger_name: tagger_name.map(|t| body.subslice_range_stable(t).unwrap()),
            tagger_email: tagger_email.map(|t| body.subslice_range_stable(t).unwrap()),
            tag_date,
            message: body.subslice_range_stable(message).unwrap(),
            additional_headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    const ZERO_OID: ObjectId = ObjectId::new([0; 20]);

    #[test]
    fn parse_commit_tag() {
        let data = b"object eedeffb6da16ddc3fb61b2255a8259cacc045691
type commit
tag annotated-tag
tagger a-user <an-email-address> 1774822895 +0100

a message
";
        let tag = Tag::parse(ZERO_OID, data.to_vec()).unwrap();
        assert_eq!(
            tag.target,
            ObjectId::new(hex!("eedeffb6da16ddc3fb61b2255a8259cacc045691"),)
        );
        assert_eq!(tag.tag_type, TagType::Commit);
        assert_eq!(tag.name(), b"annotated-tag");
        assert_eq!(tag.tagger_name(), Some(b"a-user".as_slice()));
        assert_eq!(tag.tagger_email(), Some(b"an-email-address".as_slice()));
        assert_eq!(
            tag.tag_date,
            Some(DateTime::parse_from_rfc3339("2026-03-29T23:21:35+01:00").unwrap())
        );
        assert_eq!(&tag.message(), b"a message\n");
    }

    #[test]
    fn parse_blob_tag() {
        let data = b"object e69de29bb2d1d6434b8b29ae775ad8c2e48c5391
type blob
tag blob-tag
tagger a-user <an-email-address> 1774826002 +0100

a blob
";
        let tag = Tag::parse(ZERO_OID, data.to_vec()).unwrap();
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
        let tag = Tag::parse(ZERO_OID, data.to_vec()).unwrap();
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
        let tag = Tag::parse(ZERO_OID, data.to_vec()).unwrap();
        assert_eq!(tag.tag_type, TagType::Tag);
    }
}
