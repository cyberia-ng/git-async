use crate::object::ObjectId;
use accessory::Accessors;
use alloc::vec::Vec;
#[cfg(feature = "serde")]
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Clone, Accessors)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Blob {
    #[access(get(cp))]
    id: ObjectId,

    #[access(get(ty(&[u8])))]
    #[cfg_attr(feature = "serde", serde(with = "serde_bytes"))]
    data: Vec<u8>,
}

impl Blob {
    pub(crate) fn new(id: ObjectId, data: Vec<u8>) -> Self {
        Blob { id, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Repo,
        object::{Object, ObjectType},
        test::repo::{TestRepo, TestRepoDirectory},
    };
    use nom::Parser;

    const ZERO_OID: ObjectId = ObjectId::new([0; 20]);

    fn dummy_repo() -> Repo<TestRepoDirectory> {
        TestRepo::new().unwrap().repo()
    }

    #[test]
    fn parse_empty_blob() {
        let repo = dummy_repo();
        let input = b"";
        let (_, object) = Object::parser(ZERO_OID, ObjectType::Blob, &repo)
            .parse(input)
            .unwrap();
        let blob = match object {
            Object::Blob(blob) => blob,
            _ => panic!(),
        };
        assert_eq!(blob.data, &[]);
    }

    #[test]
    fn parse_contentful_blob() {
        let repo = dummy_repo();
        let input = b"hello world";
        let (_, object) = Object::parser(ZERO_OID, ObjectType::Blob, &repo)
            .parse(input)
            .unwrap();
        let blob = match object {
            Object::Blob(blob) => blob,
            _ => panic!(),
        };
        assert_eq!(blob.data, b"hello world");
    }
}
