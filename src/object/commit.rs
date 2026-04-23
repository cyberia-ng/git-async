use crate::{
    error::GResult,
    object::{ObjectHeader, ObjectId, Tree, parse_author_committer_tagger, parse_object_headers},
    parsing::{ParseError, ParseResult},
    repo::Repo,
    traits::AllGenerics,
};
use accessory::Accessors;
use alloc::vec::Vec;
use chrono::{DateTime, FixedOffset};
use nom::{Parser, combinator::all_consuming};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Accessors, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(bound = ""))]
pub struct Commit {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get(cp))]
    tree: ObjectId,

    #[access(get(ty(&[ObjectId])))]
    parents: Vec<ObjectId>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    author_name: Vec<u8>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    author_email: Vec<u8>,

    #[access(get(cp))]
    author_date: DateTime<FixedOffset>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    committer_name: Vec<u8>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    committer_email: Vec<u8>,

    #[access(get(cp))]
    commit_date: DateTime<FixedOffset>,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "crate::serde::utf8"))]
    message: Vec<u8>,

    #[access(get(ty(&[ObjectHeader])))]
    additional_headers: Vec<ObjectHeader>,
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
    pub async fn lookup_tree<G: AllGenerics>(&self, repo: &Repo<G>) -> GResult<Tree> {
        Ok(repo.lookup_object(self.tree).await?.tree()?)
    }

    pub async fn lookup_parents<G: AllGenerics>(&self, repo: &Repo<G>) -> GResult<Vec<Commit>> {
        let mut out = Vec::with_capacity(self.parents.len());
        for parent in &self.parents {
            out.push(repo.lookup_object(*parent).await?.commit()?)
        }
        Ok(out)
    }
}

impl Commit {
    pub(crate) fn parser<'a>(id: ObjectId) -> impl Fn(&'a [u8]) -> ParseResult<&'a [u8], Self> {
        move |input: &[u8]| {
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
            let f = move || -> Option<Commit> {
                Some(Commit {
                    id,
                    author_name: author_name?,
                    author_email: author_email?,
                    author_date: author_date?,
                    committer_name: committer_name?,
                    committer_email: committer_email?,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    const ZERO_OID: ObjectId = ObjectId::new([0; 20]);

    #[test]
    fn parse_root_commit() {
        let data = b"tree 3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb
author a-user <an-email-address> 1774735018 +0530
committer another-user <another-email-address> 1774735019 -0800

a commit
";
        let (rest, commit) = Commit::parser(ZERO_OID).parse(data).unwrap();
        assert!(rest.is_empty());
        assert!(commit.parents.is_empty());
        assert_eq!(
            commit.tree,
            ObjectId::new(hex!("3a4df67dd7fd7cb3ca82d9896dbdd28053d39bdb"),)
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
        let (_, commit) = Commit::parser(ZERO_OID).parse(data).unwrap();
        assert_eq!(
            &commit.parents,
            &[ObjectId::new(hex!(
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
        let (_, commit) = Commit::parser(ZERO_OID).parse(data).unwrap();
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
        let (_, commit) = Commit::parser(ZERO_OID).parse(data).unwrap();
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
        );
    }
}
