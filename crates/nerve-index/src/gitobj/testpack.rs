//! A packfile and `.idx` **writer**, for tests only.
//!
//! `#[cfg(test)]`, and that is the point: Nerve does not write packfiles, and a pack writer in
//! product code would be a compression path with no caller. It lives here rather than in an
//! integration test so that the unit tests in [`super::pack`] and [`super::store`] share one
//! builder — two copies of a format writer is two things to keep in agreement.
//!
//! # What it can build that Git cannot
//!
//! Every hostile case in `fixtures/gitobj/expected.json` that a real `git gc` will never produce:
//! a `REF_DELTA` naming a base nothing holds, a delta chain past [`super::MAX_DELTA_DEPTH`], a
//! `REF_DELTA` whose base is itself, an entry whose declared size disagrees with its stream, and an
//! entry with the reserved type 5. Those are the cases the bounds exist for, so they have to be
//! constructible.
//!
//! # Object ids here are invented, and that is sound
//!
//! [`super::ObjectStore`] deliberately does not verify content against its id (see [`super`]), so a
//! synthetic pack can be indexed under any ids at all. That is a decision this builder depends on
//! rather than works around: if verification were ever added, these tests would be the first thing
//! to fail, which is the correct signal.

use std::io::Write;
use std::path::Path;

use super::oid::{Oid, OID_BYTES};
use super::pack::PACK_MAGIC;
use super::ObjectKind;

/// One entry to write into a synthetic pack.
pub enum Entry {
    /// A whole object, with the header size Git would write.
    Object(ObjectKind, Vec<u8>),
    /// A whole object whose header declares a size the stream does not have.
    ObjectWithDeclaredSize(ObjectKind, Vec<u8>, u64),
    /// A raw 3-bit type value, for the types that are not object types.
    RawType(u8, Vec<u8>),
    /// A delta against the entry at `base_index`, whose backward offset is computed.
    OfsDelta {
        /// Index into the entry list of the base.
        base_index: usize,
        /// The delta instruction stream, uncompressed.
        delta: Vec<u8>,
    },
    /// A delta with a backward offset written verbatim, however invalid.
    OfsDeltaRaw {
        /// The backward offset to encode.
        backward: u64,
        /// The delta instruction stream, uncompressed.
        delta: Vec<u8>,
    },
    /// A delta against an object named by id.
    RefDelta {
        /// The base's id.
        base: Oid,
        /// The delta instruction stream, uncompressed.
        delta: Vec<u8>,
    },
}

/// Where each entry landed.
pub struct Built {
    /// Byte offset of each entry, in the order they were given.
    pub offsets: Vec<u64>,
}

fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("writing to a Vec");
    encoder.finish().expect("finishing a Vec encoder")
}

/// Encode a pack entry header: 3-bit type, then the size seven bits at a time.
fn entry_header(type_id: u8, mut size: u64) -> Vec<u8> {
    let mut first = ((type_id & 0x07) << 4) | (size & 0x0f) as u8;
    size >>= 4;
    let mut out = Vec::new();
    while size != 0 {
        out.push((size & 0x7f) as u8);
        size >>= 7;
    }
    if !out.is_empty() {
        first |= 0x80;
        for index in 0..out.len() - 1 {
            out[index] |= 0x80;
        }
    }
    let mut header = vec![first];
    header.extend_from_slice(&out);
    header
}

/// Encode an `OFS_DELTA` backward offset, with the format's plus-one encoding.
fn offset_varint(value: u64) -> Vec<u8> {
    let mut bytes = vec![(value & 0x7f) as u8];
    let mut remaining = value >> 7;
    while remaining != 0 {
        remaining -= 1;
        bytes.push((remaining & 0x7f) as u8);
        remaining >>= 7;
    }
    bytes.reverse();
    let last = bytes.len() - 1;
    for (index, byte) in bytes.iter_mut().enumerate() {
        if index != last {
            *byte |= 0x80;
        }
    }
    bytes
}

/// Write a packfile at `path` and report where each entry landed.
pub fn build(path: &Path, entries: &[Entry]) -> Built {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&PACK_MAGIC);
    body.extend_from_slice(&2u32.to_be_bytes());
    body.extend_from_slice(&(entries.len() as u32).to_be_bytes());

    let mut offsets = Vec::with_capacity(entries.len());
    for entry in entries {
        let offset = body.len() as u64;
        offsets.push(offset);
        match entry {
            Entry::Object(kind, data) => {
                let type_id = pack_type_of(*kind);
                body.extend_from_slice(&entry_header(type_id, data.len() as u64));
                body.extend_from_slice(&deflate(data));
            }
            Entry::ObjectWithDeclaredSize(kind, data, declared) => {
                let type_id = pack_type_of(*kind);
                body.extend_from_slice(&entry_header(type_id, *declared));
                body.extend_from_slice(&deflate(data));
            }
            Entry::RawType(type_id, data) => {
                body.extend_from_slice(&entry_header(*type_id, data.len() as u64));
                body.extend_from_slice(&deflate(data));
            }
            Entry::OfsDelta { base_index, delta } => {
                let backward = offset - offsets[*base_index];
                body.extend_from_slice(&entry_header(6, delta.len() as u64));
                body.extend_from_slice(&offset_varint(backward));
                body.extend_from_slice(&deflate(delta));
            }
            Entry::OfsDeltaRaw { backward, delta } => {
                body.extend_from_slice(&entry_header(6, delta.len() as u64));
                body.extend_from_slice(&offset_varint(*backward));
                body.extend_from_slice(&deflate(delta));
            }
            Entry::RefDelta { base, delta } => {
                body.extend_from_slice(&entry_header(7, delta.len() as u64));
                body.extend_from_slice(base.as_bytes());
                body.extend_from_slice(&deflate(delta));
            }
        }
    }
    // The trailing SHA-1. Zeros: nothing verifies it, by decision (see `super`).
    body.extend_from_slice(&[0u8; 20]);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("a fixture directory");
    }
    std::fs::write(path, &body).expect("writing a synthetic pack");
    Built { offsets }
}

const fn pack_type_of(kind: ObjectKind) -> u8 {
    match kind {
        ObjectKind::Commit => 1,
        ObjectKind::Tree => 2,
        ObjectKind::Blob => 3,
        ObjectKind::Tag => 4,
    }
}

/// Build a well-formed `.idx` v2 mapping each `(oid, offset)`, with 32-bit offsets.
pub fn idx_bytes(entries: &[(Oid, u64)]) -> Vec<u8> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|(oid, _)| *oid);

    let mut out = Vec::new();
    out.extend_from_slice(&super::packidx::IDX_MAGIC);
    out.extend_from_slice(&super::packidx::SUPPORTED_IDX_VERSION.to_be_bytes());
    for bucket in 0..256u32 {
        let count = sorted
            .iter()
            .filter(|(oid, _)| u32::from(oid.first_byte()) <= bucket)
            .count() as u32;
        out.extend_from_slice(&count.to_be_bytes());
    }
    for (oid, _) in &sorted {
        out.extend_from_slice(oid.as_bytes());
    }
    for _ in &sorted {
        out.extend_from_slice(&0u32.to_be_bytes());
    }
    for (_, offset) in &sorted {
        out.extend_from_slice(&(*offset as u32).to_be_bytes());
    }
    out.extend_from_slice(&[0u8; 40]);
    out
}

/// Build a `.idx` **v1**: no magic, a fanout, then `u32` offset plus 20-byte id per object.
///
/// Superseded in 2007 and never written by this project for any purpose but proving that it is
/// refused *with its version stated* rather than lumped in with unreadable bytes.
pub fn idx_v1_bytes(entries: &[(Oid, u64)]) -> Vec<u8> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|(oid, _)| *oid);

    let mut out = Vec::new();
    for bucket in 0..256u32 {
        let count = sorted
            .iter()
            .filter(|(oid, _)| u32::from(oid.first_byte()) <= bucket)
            .count() as u32;
        out.extend_from_slice(&count.to_be_bytes());
    }
    for (oid, offset) in &sorted {
        out.extend_from_slice(&(*offset as u32).to_be_bytes());
        out.extend_from_slice(oid.as_bytes());
    }
    out.extend_from_slice(&[0u8; 40]);
    out
}

/// An invented object id, distinct per `seed`.
///
/// Ids are invented rather than computed because nothing verifies them; see the module
/// documentation.
pub fn fake_oid(seed: u8) -> Oid {
    let mut bytes = [0u8; OID_BYTES];
    bytes[0] = seed;
    bytes[OID_BYTES - 1] = seed;
    Oid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two varint encodings a pack uses, checked against the values the reader expects.
    #[test]
    fn the_offset_varint_encoder_and_decoder_agree() {
        for value in [0u64, 1, 127, 128, 129, 255, 256, 16_383, 16_384, 1_000_000] {
            let encoded = offset_varint(value);
            let mut cursor = 0usize;
            // Decode with the same plus-one rule the reader uses.
            let mut byte = encoded[cursor];
            cursor += 1;
            let mut decoded = u64::from(byte & 0x7f);
            while byte & 0x80 != 0 {
                byte = encoded[cursor];
                cursor += 1;
                decoded = ((decoded + 1) << 7) | u64::from(byte & 0x7f);
            }
            assert_eq!(decoded, value, "offset varint round trip for {value}");
            assert_eq!(cursor, encoded.len());
        }
    }

    #[test]
    fn the_entry_header_encoder_round_trips_a_size() {
        for size in [0u64, 1, 15, 16, 127, 128, 2047, 2048, 1_000_000] {
            let header = entry_header(3, size);
            let mut cursor = 0usize;
            let first = header[cursor];
            cursor += 1;
            assert_eq!((first >> 4) & 0x07, 3);
            let mut decoded = u64::from(first & 0x0f);
            let mut shift = 4u32;
            let mut byte = first;
            while byte & 0x80 != 0 {
                byte = header[cursor];
                cursor += 1;
                decoded |= u64::from(byte & 0x7f) << shift;
                shift += 7;
            }
            assert_eq!(decoded, size, "entry header round trip for {size}");
            assert_eq!(cursor, header.len());
        }
    }
}
