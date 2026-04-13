use crate::object::ObjectId;
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
        hex::encode_to_slice(self.id, &mut chars).unwrap();
        let encoded = str::from_utf8(&chars).unwrap();
        serializer.serialize_str(encoded)
    }
}

struct ObjectIdVisitor;
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
        deserializer.deserialize_str(ObjectIdVisitor)
    }
}

enum MaybeUtf8<'a> {
    Utf8(&'a str),
    Bytes(&'a [u8]),
}

impl<'a> From<&'a [u8]> for MaybeUtf8<'a> {
    fn from(value: &'a [u8]) -> Self {
        match str::from_utf8(value) {
            Ok(str) => Self::Utf8(str),
            Err(_) => Self::Bytes(value),
        }
    }
}

impl<'a> serde::Serialize for MaybeUtf8<'a> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use MaybeUtf8::*;
        match self {
            Utf8(str) => serializer.serialize_str(str),
            Bytes(bytes) => serializer.serialize_bytes(bytes),
        }
    }
}

pub(crate) mod utf8 {
    use super::*;

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        MaybeUtf8::from(bytes).serialize(serializer)
    }
}

pub(crate) mod option_utf8 {
    use super::*;
    use alloc::vec::Vec;

    pub fn serialize<S: Serializer>(
        bytes: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if let Some(bytes) = bytes {
            serializer.serialize_some(&MaybeUtf8::from(bytes.as_slice()))
        } else {
            serializer.serialize_none()
        }
    }
}
