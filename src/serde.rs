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
    use alloc::vec::Vec;
    use serde::de::SeqAccess;

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        MaybeUtf8::from(bytes).serialize(serializer)
    }

    struct Utf8Visitor;
    impl<'de> Visitor<'de> for Utf8Visitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut alloc::fmt::Formatter) -> alloc::fmt::Result {
            write!(formatter, "a string or byte string")
        }

        fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Self::Value, E> {
            Ok(v.to_vec())
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(v.as_bytes().to_vec())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out: Self::Value = if let Some(s) = seq.size_hint() {
                Vec::with_capacity(s)
            } else {
                Vec::new()
            };
            while let Some(element) = seq.next_element()? {
                out.push(element);
            }
            Ok(out)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        deserializer.deserialize_any(Utf8Visitor)
    }
}

pub(crate) mod option_utf8 {
    use super::*;
    use alloc::vec::Vec;
    use serde::de::SeqAccess;

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

    struct OptionUtf8Visitor;
    impl<'de> Visitor<'de> for OptionUtf8Visitor {
        type Value = Option<Vec<u8>>;

        fn expecting(&self, formatter: &mut alloc::fmt::Formatter) -> alloc::fmt::Result {
            write!(formatter, "an optional string or byte string")
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_any(OptionUtf8Visitor)
        }

        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Self::Value, E> {
            Ok(Some(v.to_vec()))
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(v.as_bytes().to_vec()))
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out: Vec<u8> = if let Some(s) = seq.size_hint() {
                Vec::with_capacity(s)
            } else {
                Vec::new()
            };
            while let Some(element) = seq.next_element()? {
                out.push(element);
            }
            Ok(Some(out))
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        deserializer.deserialize_option(OptionUtf8Visitor)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use alloc::fmt::Debug;

    fn roundtrip_json<T: Serialize + for<'de> Deserialize<'de>>(val: &T) -> T {
        let serialized = serde_json::to_string(val).unwrap();
        serde_json::from_str(&serialized).unwrap()
    }

    fn roundtrip_msgpack<T: Serialize + for<'de> Deserialize<'de>>(val: &T) -> T {
        let serialized = rmp_serde::to_vec(val).unwrap();
        rmp_serde::from_slice(&serialized).unwrap()
    }

    fn roundtrip_cbor<T: Serialize + for<'de> Deserialize<'de>>(val: &T) -> T {
        let serialized = serde_cbor::to_vec(val).unwrap();
        serde_cbor::from_slice(&serialized).unwrap()
    }

    mod utf8 {
        use super::*;

        #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
        struct TestUtf8 {
            #[serde(with = "crate::serde::utf8")]
            field: Vec<u8>,
        }

        #[test]
        fn json_non_utf8() {
            let test = TestUtf8 {
                field: vec![0xff, 0x00, 0xff, 0x00],
            };
            debug_assert!(str::from_utf8(&test.field).is_err());
            assert_eq!(test, roundtrip_json(&test));
        }

        #[test]
        fn json_utf8() {
            let test = TestUtf8 {
                field: b"hello".to_vec(),
            };
            assert_eq!(test, roundtrip_json(&test));
        }

        #[test]
        fn msgpack_non_utf8() {
            let test = TestUtf8 {
                field: vec![0xff, 0x00, 0xff, 0x00],
            };
            debug_assert!(str::from_utf8(&test.field).is_err());
            assert_eq!(test, roundtrip_msgpack(&test));
        }

        #[test]
        fn msgpack_utf8() {
            let test = TestUtf8 {
                field: b"hello".to_vec(),
            };
            assert_eq!(test, roundtrip_msgpack(&test));
        }

        #[test]
        fn cbor_non_utf8() {
            let test = TestUtf8 {
                field: vec![0xff, 0x00, 0xff, 0x00],
            };
            debug_assert!(str::from_utf8(&test.field).is_err());
            assert_eq!(test, roundtrip_cbor(&test));
        }

        #[test]
        fn cbor_utf8() {
            let test = TestUtf8 {
                field: b"hello".to_vec(),
            };
            assert_eq!(test, roundtrip_cbor(&test));
        }
    }

    mod option_utf8 {
        use super::*;

        #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
        struct TestOptionUtf8 {
            #[serde(with = "crate::serde::option_utf8")]
            field: Option<Vec<u8>>,
        }

        #[test]
        fn json_non_utf8() {
            let test = TestOptionUtf8 {
                field: Some(vec![0xff, 0x00, 0xff, 0x00]),
            };
            assert_eq!(test, roundtrip_json(&test));
        }

        #[test]
        fn json_utf8() {
            let test = TestOptionUtf8 {
                field: Some(b"hello".to_vec()),
            };
            assert_eq!(test, roundtrip_json(&test));
        }

        #[test]
        fn json_none() {
            let test = TestOptionUtf8 { field: None };
            assert_eq!(test, roundtrip_json(&test));
        }

        #[test]
        fn msgpack_non_utf8() {
            let test = TestOptionUtf8 {
                field: Some(vec![0xff, 0x00, 0xff, 0x00]),
            };
            assert_eq!(test, roundtrip_msgpack(&test));
        }

        #[test]
        fn msgpack_utf8() {
            let test = TestOptionUtf8 {
                field: Some(b"hello".to_vec()),
            };
            assert_eq!(test, roundtrip_msgpack(&test));
        }

        #[test]
        fn msgpack_none() {
            let test = TestOptionUtf8 { field: None };
            assert_eq!(test, roundtrip_msgpack(&test));
        }

        #[test]
        fn cbor_non_utf8() {
            let test = TestOptionUtf8 {
                field: Some(vec![0xff, 0x00, 0xff, 0x00]),
            };
            assert_eq!(test, roundtrip_cbor(&test));
        }

        #[test]
        fn cbor_utf8() {
            let test = TestOptionUtf8 {
                field: Some(b"hello".to_vec()),
            };
            assert_eq!(test, roundtrip_cbor(&test));
        }

        #[test]
        fn cbor_none() {
            let test = TestOptionUtf8 { field: None };
            assert_eq!(test, roundtrip_cbor(&test));
        }
    }
}
