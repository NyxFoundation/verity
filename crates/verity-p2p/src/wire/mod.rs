//! The two byte-level encodings under every lean wire format: unsigned LEB128 varints and
//! snappy in both of its formats.

pub mod snappy;
pub mod varint;
