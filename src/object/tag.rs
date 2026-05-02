use crate::{
    error::GResult,
    file_system::FileSystem,
    object::{
        Object, ObjectId, ObjectType,
        header::{ObjectHeaderIter, RangeObjectHeader},
        parse_author_committer_tagger,
    },
    parsing::ParseError,
    repo::Repo,
    subslice_range::SubsliceRange,
};
use accessory::Accessors;
use alloc::vec::Vec;
use chrono::{DateTime, FixedOffset};
use core::ops::Range;
use nom::{Parser, combinator::all_consuming};

/// A tag object
///
/// Git tags can be ref tags or tag objects; this is the latter.
#[derive(Accessors, Clone)]
pub struct Tag {
    /// The [`ObjectId`] of the tag
    #[access(get(cp))]
    id: ObjectId,

    /// The raw data in the object
    #[access(get(ty(&[u8])))]
    body: Vec<u8>,

    /// The [`ObjectId`] the object pointed to by the tag
    #[access(get(cp))]
    target: ObjectId,

    /// The type of the object pointed to by the tag
    #[allow(clippy::struct_field_names)]
    #[access(get(cp))]
    tag_type: ObjectType,

    name: Range<usize>,
    tagger_name: Option<Range<usize>>,
    tagger_email: Option<Range<usize>>,
    message: Range<usize>,

    /// The tag date, if it exists
    #[access(get(cp))]
    date: Option<DateTime<FixedOffset>>,

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
    /// The name of the tag
    pub fn name(&self) -> &[u8] {
        &self.body[self.name.clone()]
    }

    /// The name of the tagger, if specified
    pub fn tagger_name(&self) -> Option<&[u8]> {
        self.tagger_name
            .as_ref()
            .map(|range| &self.body[range.clone()])
    }

    /// The email address of the tagger, if specified
    pub fn tagger_email(&self) -> Option<&[u8]> {
        self.tagger_email
            .as_ref()
            .map(|range| &self.body[range.clone()])
    }

    /// The message of the tag
    pub fn message(&self) -> &[u8] {
        &self.body[self.message.clone()]
    }

    /// Get an iterator over any additional headers in the tag object.
    ///
    /// Additional headers are those not parsed by `git-async`, e.g. `mergetag`.
    pub fn additional_headers(&self) -> ObjectHeaderIter<'_> {
        ObjectHeaderIter::new(self.body.as_slice(), self.additional_headers.as_slice())
    }

    /// Wrap the [`Tag`] as a generic [`Object`].
    pub fn as_object(self) -> Object {
        Object::Tag(self)
    }

    /// Look up the target object of the tag using the provided [`Repo`].
    pub async fn lookup_target<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Object> {
        repo.lookup_object(self.target).await
    }

    pub(crate) fn parse(id: ObjectId, body: Vec<u8>) -> Result<Self, ParseError> {
        fn f<T>(val: Option<T>) -> Result<T, ParseError> {
            val.ok_or(ParseError::MissingFields)
        }
        let (message, raw_headers) = RangeObjectHeader::parser(&body)?;
        let mut object: Option<ObjectId> = None;
        let mut tag_type: Option<ObjectType> = None;
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
                        b"commit" => Some(ObjectType::Commit),
                        b"blob" => Some(ObjectType::Blob),
                        b"tree" => Some(ObjectType::Tree),
                        b"tag" => Some(ObjectType::Tag),
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
            date: tag_date,
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

    const ZERO_OID: ObjectId = ObjectId::from_bytes([0; 20]);

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
            ObjectId::from_bytes(hex!("eedeffb6da16ddc3fb61b2255a8259cacc045691"),)
        );
        assert_eq!(tag.tag_type, ObjectType::Commit);
        assert_eq!(tag.name(), b"annotated-tag");
        assert_eq!(tag.tagger_name(), Some(b"a-user".as_slice()));
        assert_eq!(tag.tagger_email(), Some(b"an-email-address".as_slice()));
        assert_eq!(
            tag.date,
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
        assert_eq!(tag.tag_type, ObjectType::Blob);
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
        assert_eq!(tag.tag_type, ObjectType::Tree);
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
        assert_eq!(tag.tag_type, ObjectType::Tag);
    }
}
