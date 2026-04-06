use crate::ObjectId;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error, Unexpected, Visitor},
};

impl Serialize for ObjectId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut chars = [0u8; 40];
        hex::encode_to_slice(self.0, &mut chars).unwrap();
        let encoded = str::from_utf8(&chars).unwrap();
        serializer.serialize_str(&encoded)
    }
}

struct ObjectIdVisitor {}
impl<'de> Visitor<'de> for ObjectIdVisitor {
    type Value = ObjectId;

    fn expecting(&self, formatter: &mut alloc::fmt::Formatter) -> alloc::fmt::Result {
        write!(formatter, "a 40-character hex string")
    }

    fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
        ObjectId::from_hex(v.as_bytes()).ok_or_else(|| E::invalid_value(Unexpected::Str(v), &self))
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(ObjectIdVisitor {})
    }
}

pub(crate) mod utf8 {
    use super::*;
    use alloc::vec::Vec;
    use serde::ser::Error;

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        let utf8 = str::from_utf8(&bytes).map_err(|_| S::Error::custom("invalid UTF-8 string"))?;
        serializer.serialize_str(utf8)
    }

    struct Utf8Visitor;
    impl<'de> Visitor<'de> for Utf8Visitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut alloc::fmt::Formatter) -> alloc::fmt::Result {
            write!(formatter, "a Vec<u8>")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v.as_bytes().to_vec())
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        deserializer.deserialize_str(Utf8Visitor)
    }
}
