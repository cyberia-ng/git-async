use crate::{
    error::GResult,
    file_system::FileSystem,
    object::{
        Object, ObjectId, Tree,
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

/// A commit object
#[derive(Accessors, Clone)]
pub struct Commit {
    /// The [`ObjectId`] of the commit
    #[access(get(cp))]
    id: ObjectId,

    /// The raw data in the object
    #[access(get(ty(&[u8])))]
    body: Vec<u8>,

    /// The [`ObjectId`] of the tree that the commit points to
    #[access(get(cp))]
    tree: ObjectId,

    /// The [`ObjectId`]s of all of the parents of the commit
    #[access(get(ty(&[ObjectId])))]
    parents: Vec<ObjectId>,

    author_name: Range<usize>,
    author_email: Range<usize>,
    committer_name: Range<usize>,
    committer_email: Range<usize>,
    message: Range<usize>,

    /// The author date of the commit
    #[access(get(cp))]
    author_date: DateTime<FixedOffset>,

    /// The commit date of the commit
    #[expect(clippy::struct_field_names)]
    #[access(get(cp))]
    commit_date: DateTime<FixedOffset>,

    additional_headers: Vec<RangeObjectHeader>,
}

impl PartialEq for Commit {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Commit {}
impl PartialOrd for Commit {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Commit {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl Commit {
    /// The name of the commit author
    pub fn author_name(&self) -> &[u8] {
        &self.body[self.author_name.clone()]
    }

    /// The email address of the commit author
    pub fn author_email(&self) -> &[u8] {
        &self.body[self.author_email.clone()]
    }

    /// The name of the committer
    pub fn committer_name(&self) -> &[u8] {
        &self.body[self.committer_name.clone()]
    }

    /// The email address of the committer
    pub fn committer_email(&self) -> &[u8] {
        &self.body[self.committer_email.clone()]
    }

    /// The commit message
    pub fn message(&self) -> &[u8] {
        &self.body[self.message.clone()]
    }

    /// Get an iterator over any additional headers in the commit.
    ///
    /// Additional headers are those not parsed by `git-async`, e.g. `mergetag`.
    pub fn additional_headers(&self) -> ObjectHeaderIter<'_> {
        ObjectHeaderIter::new(&self.body, &self.additional_headers)
    }

    /// Wrap the [`Commit`] as a generic [`Object`].
    pub fn as_object(self) -> Object {
        Object::Commit(self)
    }

    /// Look up the tree that the commit points to, using the provided [`Repo`].
    pub async fn lookup_tree<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Tree> {
        Ok(repo.lookup_object(self.tree).await?.tree()?)
    }

    /// Look up all the parents of the commit, using the provided [`Repo`].
    pub async fn lookup_parents<F: FileSystem>(&self, repo: &Repo<F>) -> GResult<Vec<Commit>> {
        let mut out = Vec::with_capacity(self.parents.len());
        for parent in &self.parents {
            out.push(repo.lookup_object(*parent).await?.commit()?);
        }
        Ok(out)
    }

    pub(crate) fn parse(id: ObjectId, body: Vec<u8>) -> Result<Self, ParseError> {
        fn f<T>(val: Option<T>) -> Result<T, ParseError> {
            val.ok_or(ParseError::MissingFields)
        }
        let (message, headers) = RangeObjectHeader::parser(&body)?;
        let mut tree: Option<ObjectId> = None;
        let mut parents: Vec<ObjectId> = Vec::new();
        let mut author_name: Option<&[u8]> = None;
        let mut author_email: Option<&[u8]> = None;
        let mut author_date: Option<DateTime<FixedOffset>> = None;
        let mut committer_name: Option<&[u8]> = None;
        let mut committer_email: Option<&[u8]> = None;
        let mut commit_date: Option<DateTime<FixedOffset>> = None;
        let mut additional_headers: Vec<RangeObjectHeader> = Vec::new();
        for (range_header, header) in headers
            .iter()
            .zip(ObjectHeaderIter::new(&body, headers.as_slice()))
        {
            match header.name() {
                b"tree" => {
                    let (_, object_id) = all_consuming(ObjectId::parse).parse(header.value())?;
                    tree = Some(object_id);
                }
                b"parent" => {
                    let (_, object_id) = all_consuming(ObjectId::parse).parse(header.value())?;
                    parents.push(object_id);
                }
                b"author" => {
                    let (_, (name, email, date)) =
                        all_consuming(parse_author_committer_tagger).parse(header.value())?;
                    author_name = Some(name);
                    author_email = Some(email);
                    author_date = Some(date);
                }
                b"committer" => {
                    let (_, (name, email, date)) =
                        all_consuming(parse_author_committer_tagger).parse(header.value())?;
                    committer_name = Some(name);
                    committer_email = Some(email);
                    commit_date = Some(date);
                }
                _ => {
                    additional_headers.push(range_header.clone());
                }
            }
        }
        Ok(Self {
            id,
            message: body.subslice_range_stable(message).unwrap(),
            tree: f(tree)?,
            parents,
            author_name: body.subslice_range_stable(f(author_name)?).unwrap(),
            author_email: body.subslice_range_stable(f(author_email)?).unwrap(),
            author_date: f(author_date)?,
            committer_name: body.subslice_range_stable(f(committer_name)?).unwrap(),
            committer_email: body.subslice_range_stable(f(committer_email)?).unwrap(),
            commit_date: f(commit_date)?,
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
    fn parse_root_commit() {
        let data = b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a-user <an-email-address> 1774735018 +0530
committer another-user <another-email-address> 1774735019 -0800

a commit
";
        let commit = Commit::parse(ZERO_OID, data.to_vec()).unwrap();
        assert!(commit.parents.is_empty());
        assert_eq!(
            commit.tree,
            ObjectId::from_bytes(hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb"),)
        );
        assert_eq!(str::from_utf8(commit.author_name()).unwrap(), "a-user");
        assert_eq!(
            str::from_utf8(commit.author_email()).unwrap(),
            "an-email-address"
        );
        assert_eq!(
            commit.author_date,
            DateTime::parse_from_rfc3339("2026-03-29T03:26:58+05:30").unwrap()
        );
        assert_eq!(
            str::from_utf8(commit.committer_name()).unwrap(),
            "another-user"
        );
        assert_eq!(
            str::from_utf8(commit.committer_email()).unwrap(),
            "another-email-address"
        );
        assert_eq!(
            commit.commit_date,
            DateTime::parse_from_rfc3339("2026-03-28T13:56:59-08:00").unwrap()
        );
        assert_eq!(str::from_utf8(commit.message()).unwrap(), "a commit\n");
    }

    #[test]
    fn parse_normal_commit() {
        let data = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904
parent 16dafd3d0ba5af72f035d641c076a4150eda548d
author a-user <an-email-address> 1774739676 +0000
committer a-user <an-email-address> 1774739676 +0000

another commit
";
        let commit = Commit::parse(ZERO_OID, data.to_vec()).unwrap();
        assert_eq!(
            &commit.parents,
            &[ObjectId::from_bytes(hex!(
                "16dafd3d0ba5af72f035d641c076a4150eda548d"
            ),)]
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
        let commit = Commit::parse(ZERO_OID, data.to_vec()).unwrap();
        assert_eq!(commit.parents.len(), 2);
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
        let commit = Commit::parse(ZERO_OID, data.to_vec()).unwrap();
        let expected = [
            (b"some-header".as_slice(), b"a value".as_slice()),
            (
                b"some-other-header".as_slice(),
                b"a long line-wrapped\n  value".as_slice(),
            ),
        ];
        let iter = commit.additional_headers();
        assert_eq!(iter.len(), 2);
        for (received, (expected_name, expected_value)) in iter.zip(expected.into_iter()) {
            assert_eq!(received.name(), expected_name);
            assert_eq!(received.value(), expected_value);
        }
    }
}
