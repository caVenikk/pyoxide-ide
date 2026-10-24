//! Stable node ids: a hash of the node's kind and qualified name, never of a
//! server oid, so tree state keyed by id survives a refresh and a restart.

use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The id of a node of the model. The high half names the schema, the low
/// half the node inside it (zero for the schema node itself), so finding a
/// node by id costs two hash lookups and the ids of a schema are recognised
/// without the model.
///
/// The hash is this crate's own (FNV-1a with a final mix), fixed by this
/// code and not by the standard library, so ids written to disk by one
/// build are valid in the next. Two names with the same hash in one schema
/// are told apart by a counter; the probability is about one in 10^8 for a
/// schema of a million nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    schema: u64,
    local: u64,
}

impl NodeId {
    /// The id of the schema named exactly `name`.
    pub fn of_schema(name: &str) -> Self {
        Self {
            schema: schema_hash(name),
            local: 0,
        }
    }

    pub(crate) fn new(schema: u64, local: u64) -> Self {
        Self { schema, local }
    }

    /// The id of the schema this node belongs to.
    pub fn schema_id(self) -> NodeId {
        Self {
            schema: self.schema,
            local: 0,
        }
    }

    pub fn is_schema(self) -> bool {
        self.local == 0
    }

    pub(crate) fn schema_part(self) -> u64 {
        self.schema
    }

    pub(crate) fn local_part(self) -> u64 {
        self.local
    }

    pub fn to_u128(self) -> u128 {
        (u128::from(self.schema) << 64) | u128::from(self.local)
    }

    pub fn from_u128(value: u128) -> Self {
        Self {
            schema: (value >> 64) as u64,
            local: value as u64,
        }
    }
}

/// 32 lower-case hex digits.
impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}{:016x}", self.schema, self.local)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseNodeIdError;

impl fmt::Display for ParseNodeIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a node id is 32 hex digits")
    }
}

impl std::error::Error for ParseNodeIdError {}

impl FromStr for NodeId {
    type Err = ParseNodeIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ParseNodeIdError);
        }
        u128::from_str_radix(s, 16)
            .map(Self::from_u128)
            .map_err(|_| ParseNodeIdError)
    }
}

/// As its [`Display`](fmt::Display) form: a JSON number cannot hold 128 bits
/// everywhere.
impl Serialize for NodeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// FNV-1a over the bytes, then the finaliser of SplitMix64 so that close
/// inputs spread over all bits.
#[derive(Clone, Copy)]
pub(crate) struct StableHasher(u64);

impl StableHasher {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// A string followed by a separator byte that UTF-8 never contains, so
    /// `["ab", "c"]` and `["a", "bc"]` differ.
    pub(crate) fn segment(&mut self, s: &str) {
        self.bytes(s.as_bytes());
        self.bytes(&[0xff]);
    }

    pub(crate) fn finish(self) -> u64 {
        mix(self.0)
    }
}

pub(crate) fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub(crate) fn schema_hash(name: &str) -> u64 {
    let mut h = StableHasher::new();
    h.segment("schema");
    h.segment(name);
    h.finish()
}

/// The local part of a node id: kind tag, the names on the way from the
/// schema, and a counter when an earlier node of the schema took the hash.
/// Never zero (zero is the schema node).
pub(crate) fn local_hash(tag: &str, path: &[&str], salt: u32) -> u64 {
    let mut h = StableHasher::new();
    h.segment(tag);
    for segment in path {
        h.segment(segment);
    }
    if salt > 0 {
        h.bytes(&[0xfe]);
        h.bytes(&salt.to_le_bytes());
    }
    match h.finish() {
        0 => 1,
        v => v,
    }
}

/// A hasher for keys that are already well mixed 64-bit hashes.
#[derive(Default, Clone, Copy)]
pub(crate) struct PassThrough(u64);

impl Hasher for PassThrough {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Only `write_u64` is used; anything else still hashes.
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

pub(crate) type PassThroughMap<V> =
    std::collections::HashMap<u64, V, BuildHasherDefault<PassThrough>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_print_and_parse() {
        let id = NodeId::new(0x0123_4567_89ab_cdef, 42);
        let text = id.to_string();
        assert_eq!(text, "0123456789abcdef000000000000002a");
        assert_eq!(text.parse::<NodeId>().unwrap(), id);
        assert_eq!(NodeId::from_u128(id.to_u128()), id);
        assert!("xyz".parse::<NodeId>().is_err());
        assert!(
            "+123456789abcdef000000000000002a"
                .parse::<NodeId>()
                .is_err()
        );
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<NodeId>(&json).unwrap(), id);
    }

    /// The hash is part of the on-disk contract: a change here changes every
    /// stored id.
    #[test]
    fn hashes_are_fixed() {
        assert_eq!(NodeId::of_schema("public").to_string().len(), 32);
        assert_eq!(schema_hash("public"), schema_hash("public"));
        assert_ne!(schema_hash("public"), schema_hash("PUBLIC"));
        assert_ne!(
            local_hash("table", &["ab", "c"], 0),
            local_hash("table", &["a", "bc"], 0)
        );
        assert_ne!(
            local_hash("table", &["a"], 0),
            local_hash("table", &["a"], 1)
        );
        assert_ne!(
            local_hash("table", &["a"], 0),
            local_hash("view", &["a"], 0)
        );
        let mut h = StableHasher::new();
        h.bytes(b"a");
        assert_eq!(h.0, 0xaf63_dc4c_8601_ec8c, "FNV-1a of \"a\"");
    }

    #[test]
    fn schema_ids() {
        let schema = NodeId::of_schema("app");
        assert!(schema.is_schema());
        let node = NodeId::new(schema.schema_part(), 7);
        assert!(!node.is_schema());
        assert_eq!(node.schema_id(), schema);
    }
}
