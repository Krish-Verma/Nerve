//! Pack entry headers, and `OFS_DELTA` / `REF_DELTA` reconstruction.
//!
//! # The entry header
//!
//! ```text
//!   byte 0: [continuation:1][type:3][size low 4 bits:4]
//!   byte n: [continuation:1][size next 7 bits:7]        …little-endian, 7 bits at a time
//! ```
//!
//! Types `1..=4` are commit, tree, blob and tag. `0` is invalid and `5` has never been assigned;
//! both are refused rather than mapped onto anything. `6` is `OFS_DELTA`, followed by a **backward**
//! offset in a second, differently-encoded varint; `7` is `REF_DELTA`, followed by the base's
//! 20-byte object id.
//!
//! The two varints in an entry are not the same encoding, and conflating them is the classic way to
//! misread a pack. The size varint is a plain little-endian 7-bits-at-a-time accumulation. The
//! `OFS_DELTA` offset varint adds one to the accumulator before each shift, so that every value has
//! exactly one representation — `((n + 1) << 7) | (byte & 0x7f)`.
//!
//! # Reconstruction, and why the depth bound is the only cycle control
//!
//! An `OFS_DELTA` names its base by a strictly *backward* offset, which is enforced here, so an
//! `OFS_DELTA` chain terminates by construction. A `REF_DELTA` names its base by object id and can
//! therefore point anywhere, including at itself. [`MAX_DELTA_DEPTH`] catches that, and it is the
//! *only* control for it: the bound is needed regardless — Git's own default `pack.depth` is 50 —
//! and adding cycle detection alongside it would be two mechanisms for one hazard, each able to rot
//! independently.
//!
//! Every intermediate reconstruction in a chain is bounded by
//! [`super::inflate::MAX_OBJECT_BYTES`] as well, so a chain of legal-looking deltas cannot amplify
//! past the bound by accumulating.

use std::cell::{Cell, RefCell};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::inflate::{inflate_bounded, MAX_OBJECT_BYTES};
use super::oid::{Oid, OID_BYTES};
use super::{Error, ObjectKind, Result};

/// The four bytes a packfile begins with.
pub const PACK_MAGIC: [u8; 4] = *b"PACK";

/// Pack versions this reader accepts. Version 3 exists and is byte-compatible for entry reading.
pub const SUPPORTED_PACK_VERSIONS: [u32; 2] = [2, 3];

/// The deepest delta chain this reader will follow.
///
/// Bounded at 64. Git's own default `pack.depth` is 50, so every chain Git writes is inside this and
/// a chain past it is refused rather than followed. It doubles as the cycle control for `REF_DELTA` —
/// see the module documentation for why there is deliberately no second mechanism.
pub const MAX_DELTA_DEPTH: usize = 64;

/// Bytes of an entry header this reader will read before giving up.
///
/// Derived: a size varint is at most 10 bytes for a `u64`, an `OFS_DELTA` offset varint at most 10,
/// and a `REF_DELTA` base id is 20. 64 covers the largest legal header with room to spare.
const MAX_ENTRY_HEADER_BYTES: usize = 64;

/// Bytes of pack header before the first entry: magic, version, object count.
const PACK_HEADER_BYTES: u64 = 12;

/// The pack's trailing SHA-1. No entry's data extends into it.
const PACK_TRAILER_BYTES: u64 = 20;

/// One entry as the pack stores it, before any delta is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackEntry {
    /// A whole object.
    Object {
        /// Which of the four types.
        kind: ObjectKind,
        /// The object's content bytes.
        data: Vec<u8>,
    },
    /// A delta against an object earlier in this same pack.
    OfsDelta {
        /// The base's offset in this pack. Always strictly less than this entry's own offset.
        base_offset: u64,
        /// The delta instruction stream.
        delta: Vec<u8>,
    },
    /// A delta against an object named by id, which may be in any pack or loose, or absent.
    RefDelta {
        /// The base's object id.
        base: Oid,
        /// The delta instruction stream.
        delta: Vec<u8>,
    },
}

/// An open packfile, read by offset.
///
/// The file is held open and seeked rather than read whole: a real pack is routinely hundreds of
/// megabytes and every lookup arrives with an offset from the `.idx`, so there is never a reason to
/// hold the whole thing. The [`RefCell`] is what lets [`read_entry`](Self::read_entry) take `&self`,
/// which in turn is what lets [`super::ObjectStore::read`] take `&self`.
#[derive(Debug)]
pub struct PackFile {
    path: PathBuf,
    file: RefCell<std::fs::File>,
    length: u64,
    object_count: u32,
}

impl PackFile {
    /// Open and validate a packfile's header.
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        if length < PACK_HEADER_BYTES + PACK_TRAILER_BYTES {
            return Err(Error::PackTruncated {
                offset: 0,
                wanted: (PACK_HEADER_BYTES + PACK_TRAILER_BYTES) as usize,
                length,
            });
        }
        let pack = Self {
            path: path.to_path_buf(),
            file: RefCell::new(file),
            length,
            object_count: 0,
        };
        let mut header = [0u8; 12];
        pack.read_exact_at(0, &mut header)?;
        if header[..4] != PACK_MAGIC {
            return Err(Error::PackBadMagic);
        }
        let version = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        if !SUPPORTED_PACK_VERSIONS.contains(&version) {
            return Err(Error::PackUnsupportedVersion(version));
        }
        let object_count = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
        Ok(Self {
            object_count,
            ..pack
        })
    }

    /// Where this pack lives.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The pack's length in bytes, including its trailing checksum.
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// How many objects the pack header claims.
    pub const fn object_count(&self) -> u32 {
        self.object_count
    }

    /// The first offset past every entry's data — the start of the trailing checksum.
    const fn data_end(&self) -> u64 {
        self.length - PACK_TRAILER_BYTES
    }

    /// Fill `buf` from `offset`, refusing rather than reading into the trailer or past the end.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|end| end > self.length)
        {
            return Err(Error::PackTruncated {
                offset,
                wanted: buf.len(),
                length: self.length,
            });
        }
        let mut file = self.file.borrow_mut();
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buf).map_err(|err| {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::PackTruncated {
                    offset,
                    wanted: buf.len(),
                    length: self.length,
                }
            } else {
                Error::Io(err)
            }
        })
    }

    /// Read as much of `buf` as the entry region holds, starting at `offset`.
    fn read_up_to_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let end = self.data_end();
        if offset >= end {
            return Err(Error::PackTruncated {
                offset,
                wanted: 1,
                length: self.length,
            });
        }
        let want = ((end - offset) as usize).min(buf.len());
        self.read_exact_at(offset, &mut buf[..want])?;
        Ok(want)
    }

    /// Inflate the compressed stream that begins at `offset`.
    ///
    /// The underlying reader is limited to the entry region, so a stream whose deflate data runs
    /// past the end of the pack is reported as [`Error::PackTruncated`] rather than as a generic
    /// inflate failure: those are different findings and a truncated pack is the common one.
    fn inflate_at(&self, offset: u64, limit: usize) -> Result<Vec<u8>> {
        let end = self.data_end();
        if offset > end {
            return Err(Error::PackTruncated {
                offset,
                wanted: 1,
                length: self.length,
            });
        }
        let available = end - offset;
        let consumed = Cell::new(0u64);
        let outcome = {
            let mut file = self.file.borrow_mut();
            file.seek(SeekFrom::Start(offset))?;
            let limited = (&mut *file).take(available);
            let counting = Counting {
                inner: limited,
                consumed: &consumed,
            };
            inflate_bounded(BufReader::new(counting), limit)
        };
        match outcome {
            Ok(data) => Ok(data),
            // The stream asked for every byte the pack had left and still wanted more, which is a
            // truncated pack rather than corrupt deflate data.
            Err(Error::Inflate(_)) if consumed.get() >= available => Err(Error::PackTruncated {
                offset,
                wanted: (available + 1) as usize,
                length: self.length,
            }),
            Err(other) => Err(other),
        }
    }

    /// Read the entry at `offset`, without resolving any delta.
    pub fn read_entry(&self, offset: u64) -> Result<PackEntry> {
        if offset < PACK_HEADER_BYTES {
            return Err(Error::OfsDeltaBadOffset { offset });
        }
        let mut header = [0u8; MAX_ENTRY_HEADER_BYTES];
        let available = self.read_up_to_at(offset, &mut header)?;
        let header = &header[..available];

        let mut cursor = 0usize;
        let first = next_byte(header, &mut cursor, offset, self.length)?;
        let type_id = (first >> 4) & 0x07;
        let mut size = u64::from(first & 0x0f);
        let mut shift = 4u32;
        let mut byte = first;
        while byte & 0x80 != 0 {
            byte = next_byte(header, &mut cursor, offset, self.length)?;
            let chunk = u64::from(byte & 0x7f);
            if shift >= 64 {
                if chunk != 0 {
                    return Err(Error::ObjectTooLarge {
                        limit: MAX_OBJECT_BYTES,
                        at_least: usize::MAX,
                    });
                }
            } else {
                size |= chunk << shift;
            }
            shift += 7;
        }
        if size > MAX_OBJECT_BYTES as u64 {
            return Err(Error::ObjectTooLarge {
                limit: MAX_OBJECT_BYTES,
                at_least: usize::try_from(size).unwrap_or(usize::MAX),
            });
        }

        match type_id {
            6 => {
                let backward = read_offset_varint(header, &mut cursor, offset, self.length)?;
                // Strictly backward, strictly inside the entry region. `checked_sub` plus the
                // header floor is what makes an OFS_DELTA chain terminate by construction.
                let base_offset = offset
                    .checked_sub(backward)
                    .filter(|base| backward > 0 && *base >= PACK_HEADER_BYTES)
                    .ok_or(Error::OfsDeltaBadOffset { offset })?;
                let delta = self.entry_data(offset, cursor, size)?;
                Ok(PackEntry::OfsDelta { base_offset, delta })
            }
            7 => {
                if cursor + OID_BYTES > header.len() {
                    return Err(Error::PackTruncated {
                        offset,
                        wanted: cursor + OID_BYTES,
                        length: self.length,
                    });
                }
                let base =
                    Oid::from_slice(&header[cursor..cursor + OID_BYTES]).expect("a 20-byte slice");
                cursor += OID_BYTES;
                let delta = self.entry_data(offset, cursor, size)?;
                Ok(PackEntry::RefDelta { base, delta })
            }
            other => {
                let kind =
                    ObjectKind::from_pack_type(other).ok_or(Error::PackEntryUnknownType(other))?;
                let data = self.entry_data(offset, cursor, size)?;
                Ok(PackEntry::Object { kind, data })
            }
        }
    }

    /// Inflate an entry's payload and check it against the size the header declared.
    ///
    /// The same discipline as a loose object's header: the format states the size twice, and a
    /// disagreement is refused rather than resolved in either direction.
    fn entry_data(&self, offset: u64, header_len: usize, declared: u64) -> Result<Vec<u8>> {
        let data = self.inflate_at(offset + header_len as u64, MAX_OBJECT_BYTES)?;
        if data.len() as u64 != declared {
            return Err(Error::PackEntrySizeDisagrees {
                declared,
                actual: data.len(),
            });
        }
        Ok(data)
    }
}

/// A `Read` that records how many bytes it handed over, so a stream that exhausted the pack can be
/// told apart from one that was simply corrupt.
struct Counting<'a, R> {
    inner: R,
    consumed: &'a Cell<u64>,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.consumed.set(self.consumed.get() + read as u64);
        Ok(read)
    }
}

fn next_byte(header: &[u8], cursor: &mut usize, offset: u64, length: u64) -> Result<u8> {
    let byte = *header.get(*cursor).ok_or(Error::PackTruncated {
        offset,
        wanted: *cursor + 1,
        length,
    })?;
    *cursor += 1;
    Ok(byte)
}

/// Read an `OFS_DELTA` backward offset.
///
/// **Not** the same encoding as the size varint. Each continuation adds one before shifting, so
/// every offset has exactly one representation; reading it as a plain varint yields a base offset
/// that is wrong by a growing amount and lands on a byte that is not an entry header.
fn read_offset_varint(header: &[u8], cursor: &mut usize, offset: u64, length: u64) -> Result<u64> {
    let mut byte = next_byte(header, cursor, offset, length)?;
    let mut value = u64::from(byte & 0x7f);
    while byte & 0x80 != 0 {
        byte = next_byte(header, cursor, offset, length)?;
        // `checked_mul(128)`, not `checked_shl(7)`: `checked_shl` validates the *shift amount* and
        // returns `Some` for any shift under 64, so it would happily discard the high bits of an
        // overflowing offset and produce a plausible-looking base offset from an absurd one.
        value = value
            .checked_add(1)
            .and_then(|value| value.checked_mul(128))
            .and_then(|value| value.checked_add(u64::from(byte & 0x7f)))
            .ok_or(Error::OfsDeltaBadOffset { offset })?;
    }
    Ok(value)
}

/// Read a plain little-endian 7-bits-at-a-time varint from a delta header.
fn read_size_varint(delta: &[u8], cursor: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *delta
            .get(*cursor)
            .ok_or_else(|| Error::DeltaMalformed("size varint ran off the end".to_string()))?;
        *cursor += 1;
        let chunk = u64::from(byte & 0x7f);
        if shift >= 64 {
            if chunk != 0 {
                return Err(Error::DeltaMalformed(
                    "size varint does not fit in 64 bits".to_string(),
                ));
            }
        } else {
            value |= chunk << shift;
        }
        shift += 7;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
}

/// Apply a delta instruction stream to `base`.
///
/// ```text
///   <base size varint><result size varint>
///   then, until the stream ends:
///     0x80 | mask   copy from base; the mask selects which offset and size bytes follow
///     0x01..=0x7f   that many literal bytes follow
///     0x00          invalid
/// ```
///
/// Both declared sizes are checked — the base size against the base actually supplied, and the
/// result size against what the instructions produce — and the output is bounded during the loop
/// rather than only at the end, so a stream that would overrun the declared result is refused before
/// the bytes are copied.
pub fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>> {
    let mut cursor = 0usize;
    let declared_base = read_size_varint(delta, &mut cursor)?;
    if declared_base != base.len() as u64 {
        return Err(Error::DeltaBaseSizeDisagrees {
            declared: declared_base,
            actual: base.len(),
        });
    }
    let declared_result = read_size_varint(delta, &mut cursor)?;
    if declared_result > MAX_OBJECT_BYTES as u64 {
        return Err(Error::ObjectTooLarge {
            limit: MAX_OBJECT_BYTES,
            at_least: usize::try_from(declared_result).unwrap_or(usize::MAX),
        });
    }

    // Not pre-allocated to the declared size: a twenty-byte delta claiming a 64 MiB result would
    // otherwise cost 64 MiB before a single instruction ran.
    let mut out: Vec<u8> = Vec::new();
    while cursor < delta.len() {
        let opcode = delta[cursor];
        cursor += 1;
        if opcode == 0 {
            return Err(Error::DeltaMalformed("opcode 0 is not defined".to_string()));
        }
        if opcode & 0x80 != 0 {
            let mut copy_offset = 0u64;
            for index in 0..4u32 {
                if opcode & (1 << index) != 0 {
                    let byte = *delta.get(cursor).ok_or_else(|| {
                        Error::DeltaMalformed("copy offset ran off the end".to_string())
                    })?;
                    cursor += 1;
                    copy_offset |= u64::from(byte) << (8 * index);
                }
            }
            let mut copy_size = 0u64;
            for index in 0..3u32 {
                if opcode & (0x10 << index) != 0 {
                    let byte = *delta.get(cursor).ok_or_else(|| {
                        Error::DeltaMalformed("copy size ran off the end".to_string())
                    })?;
                    cursor += 1;
                    copy_size |= u64::from(byte) << (8 * index);
                }
            }
            // The format's own special case: a zero size means 0x10000, because three bytes cannot
            // express 65536 and that is the chunk size Git's delta compressor prefers.
            if copy_size == 0 {
                copy_size = 0x10000;
            }
            let end = copy_offset
                .checked_add(copy_size)
                .filter(|end| *end <= base.len() as u64)
                .ok_or_else(|| {
                    Error::DeltaMalformed(format!(
                        "copy of {copy_size} at {copy_offset} is outside the {}-byte base",
                        base.len()
                    ))
                })?;
            if out.len() as u64 + copy_size > declared_result {
                return Err(Error::DeltaResultSizeDisagrees {
                    declared: declared_result,
                    actual: out.len() + copy_size as usize,
                });
            }
            out.extend_from_slice(&base[copy_offset as usize..end as usize]);
        } else {
            let length = usize::from(opcode & 0x7f);
            let end = cursor.checked_add(length).filter(|end| *end <= delta.len());
            let end = end
                .ok_or_else(|| Error::DeltaMalformed("literal run ran off the end".to_string()))?;
            if out.len() as u64 + length as u64 > declared_result {
                return Err(Error::DeltaResultSizeDisagrees {
                    declared: declared_result,
                    actual: out.len() + length,
                });
            }
            out.extend_from_slice(&delta[cursor..end]);
            cursor = end;
        }
    }

    if out.len() as u64 != declared_result {
        return Err(Error::DeltaResultSizeDisagrees {
            declared: declared_result,
            actual: out.len(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitobj::form;
    use crate::gitobj::testpack;

    fn size_varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return out;
            }
        }
    }

    /// `<base size><result size>` followed by raw instructions.
    fn delta_of(base_size: u64, result_size: u64, instructions: &[u8]) -> Vec<u8> {
        let mut out = size_varint(base_size);
        out.extend_from_slice(&size_varint(result_size));
        out.extend_from_slice(instructions);
        out
    }

    #[test]
    fn a_literal_only_delta_reproduces_its_literals() {
        let base = b"".to_vec();
        let delta = delta_of(0, 5, &[0x05, b'h', b'e', b'l', b'l', b'o']);
        assert_eq!(apply_delta(&base, &delta).unwrap(), b"hello");
    }

    #[test]
    fn a_copy_instruction_copies_from_the_base() {
        let base = b"0123456789".to_vec();
        // 0x80 | 0x01 (one offset byte) | 0x10 (one size byte): copy 4 bytes from offset 3.
        let delta = delta_of(10, 4, &[0x91, 0x03, 0x04]);
        assert_eq!(apply_delta(&base, &delta).unwrap(), b"3456");
    }

    #[test]
    fn copy_and_literal_instructions_interleave() {
        let base = b"0123456789".to_vec();
        let delta = delta_of(10, 7, &[0x91, 0x00, 0x03, 0x04, b'X', b'Y', b'Z', b'W']);
        assert_eq!(apply_delta(&base, &delta).unwrap(), b"012XYZW");
    }

    /// A zero copy size means 0x10000; the reader must not treat it as an empty copy.
    #[test]
    fn a_zero_copy_size_means_sixty_five_thousand_five_hundred_and_thirty_six() {
        let base = vec![b'z'; 0x10000];
        let delta = delta_of(0x10000, 0x10000, &[0x80]);
        let out = apply_delta(&base, &delta).unwrap();
        assert_eq!(out.len(), 0x10000);
        assert!(out.iter().all(|byte| *byte == b'z'));
    }

    #[test]
    fn a_zero_opcode_is_refused() {
        let error = apply_delta(b"", &delta_of(0, 0, &[0x00])).unwrap_err();
        assert_eq!(error.form(), form::DELTA_MALFORMED);
    }

    #[test]
    fn a_copy_outside_the_base_is_refused_rather_than_clamped() {
        let base = b"0123".to_vec();
        // Copy 200 bytes from offset 2 of a 4-byte base.
        let error = apply_delta(&base, &delta_of(4, 200, &[0x91, 0x02, 0xc8])).unwrap_err();
        assert_eq!(error.form(), form::DELTA_MALFORMED);

        // And an offset that overflows rather than merely exceeding.
        let error = apply_delta(
            &base,
            &delta_of(4, 4, &[0x8f, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0xff]),
        )
        .unwrap_err();
        assert_eq!(error.form(), form::DELTA_MALFORMED);
    }

    #[test]
    fn a_literal_run_past_the_end_of_the_delta_is_refused() {
        let error = apply_delta(b"", &delta_of(0, 9, &[0x09, b'a'])).unwrap_err();
        assert_eq!(error.form(), form::DELTA_MALFORMED);
    }

    #[test]
    fn a_base_size_that_disagrees_with_the_base_is_refused() {
        let error = apply_delta(b"0123", &delta_of(99, 1, &[0x01, b'x'])).unwrap_err();
        match error {
            Error::DeltaBaseSizeDisagrees { declared, actual } => {
                assert_eq!(declared, 99);
                assert_eq!(actual, 4);
            }
            other => panic!("expected a base size disagreement, got {other:?}"),
        }
    }

    #[test]
    fn a_result_size_that_disagrees_with_what_was_produced_is_refused() {
        // Too little.
        let error = apply_delta(b"", &delta_of(0, 9, &[0x01, b'x'])).unwrap_err();
        assert_eq!(error.form(), form::DELTA_RESULT_SIZE_DISAGREES);
        // Too much, refused during the loop rather than after the copy.
        let error = apply_delta(b"", &delta_of(0, 1, &[0x03, b'x', b'y', b'z'])).unwrap_err();
        assert_eq!(error.form(), form::DELTA_RESULT_SIZE_DISAGREES);
    }

    #[test]
    fn a_delta_declaring_a_result_past_the_object_bound_is_refused_before_it_runs() {
        let error = apply_delta(b"", &delta_of(0, MAX_OBJECT_BYTES as u64 + 1, &[])).unwrap_err();
        assert_eq!(error.form(), form::OBJECT_TOO_LARGE);
    }

    #[test]
    fn a_truncated_delta_header_is_refused() {
        assert_eq!(
            apply_delta(b"", &[]).unwrap_err().form(),
            form::DELTA_MALFORMED
        );
        assert_eq!(
            apply_delta(b"", &[0x80]).unwrap_err().form(),
            form::DELTA_MALFORMED
        );
    }

    // ---- the offset varint, which is not the size varint ---------------------------------------

    #[test]
    fn the_offset_varint_uses_the_plus_one_encoding() {
        // One byte: the value is the low seven bits.
        let mut cursor = 0;
        assert_eq!(
            read_offset_varint(&[0x7f], &mut cursor, 100, 1000).unwrap(),
            127
        );
        // Two bytes: ((0x01 + 1) << 7) | 0x00 == 256. A plain varint would read 128.
        let mut cursor = 0;
        assert_eq!(
            read_offset_varint(&[0x81, 0x00], &mut cursor, 100, 1000).unwrap(),
            256
        );
        let mut cursor = 0;
        assert_eq!(
            read_size_varint(&[0x80, 0x01], &mut cursor).unwrap(),
            128,
            "the two encodings must not be conflated"
        );
    }

    #[test]
    fn an_offset_varint_that_overflows_is_refused() {
        let mut cursor = 0;
        let error = read_offset_varint(&[0xff; 16], &mut cursor, 100, 1000).unwrap_err();
        assert_eq!(error.form(), form::OFS_DELTA_BAD_OFFSET);
    }

    // ---- whole packs, built here ---------------------------------------------------------------

    #[test]
    fn a_pack_of_whole_objects_reads_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        let built = testpack::build(
            &path,
            &[
                testpack::Entry::Object(ObjectKind::Blob, b"hello".to_vec()),
                testpack::Entry::Object(ObjectKind::Commit, b"tree abc\n".to_vec()),
            ],
        );
        let pack = PackFile::open(&path).unwrap();
        assert_eq!(pack.object_count(), 2);
        assert_eq!(pack.path(), path);
        assert_eq!(
            pack.read_entry(built.offsets[0]).unwrap(),
            PackEntry::Object {
                kind: ObjectKind::Blob,
                data: b"hello".to_vec()
            }
        );
        assert_eq!(
            pack.read_entry(built.offsets[1]).unwrap(),
            PackEntry::Object {
                kind: ObjectKind::Commit,
                data: b"tree abc\n".to_vec()
            }
        );
    }

    #[test]
    fn an_ofs_delta_entry_reports_a_backward_base_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        let built = testpack::build(
            &path,
            &[
                testpack::Entry::Object(ObjectKind::Blob, b"0123456789".to_vec()),
                testpack::Entry::OfsDelta {
                    base_index: 0,
                    delta: delta_of(10, 4, &[0x91, 0x03, 0x04]),
                },
            ],
        );
        let pack = PackFile::open(&path).unwrap();
        match pack.read_entry(built.offsets[1]).unwrap() {
            PackEntry::OfsDelta { base_offset, delta } => {
                assert_eq!(base_offset, built.offsets[0]);
                assert_eq!(apply_delta(b"0123456789", &delta).unwrap(), b"3456");
            }
            other => panic!("expected an OFS_DELTA, got {other:?}"),
        }
    }

    #[test]
    fn a_ref_delta_entry_reports_its_base_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        let base = Oid::from_hex("18156416b5128537cb41d292619c182151aa5dad").unwrap();
        let built = testpack::build(
            &path,
            &[testpack::Entry::RefDelta {
                base,
                delta: delta_of(0, 1, &[0x01, b'x']),
            }],
        );
        let pack = PackFile::open(&path).unwrap();
        match pack.read_entry(built.offsets[0]).unwrap() {
            PackEntry::RefDelta { base: found, .. } => assert_eq!(found, base),
            other => panic!("expected a REF_DELTA, got {other:?}"),
        }
    }

    #[test]
    fn a_pack_with_the_wrong_magic_or_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        testpack::build(
            &path,
            &[testpack::Entry::Object(ObjectKind::Blob, b"x".to_vec())],
        );

        let mut bytes = std::fs::read(&path).unwrap();
        bytes[..4].copy_from_slice(b"KCAP");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            PackFile::open(&path).unwrap_err().form(),
            form::PACK_BAD_MAGIC
        );

        bytes[..4].copy_from_slice(b"PACK");
        bytes[4..8].copy_from_slice(&9u32.to_be_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            PackFile::open(&path).unwrap_err().form(),
            form::PACK_UNSUPPORTED_VERSION
        );
    }

    /// Content that deflate cannot shrink, so the entry's compressed stream is long enough that a
    /// cut lands inside it rather than past the whole pack.
    fn incompressible(length: usize) -> Vec<u8> {
        let mut state = 0x1234_5678_9abc_def0u64;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn a_pack_truncated_mid_entry_is_refused_with_no_partial_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        let built = testpack::build(
            &path,
            &[testpack::Entry::Object(
                ObjectKind::Blob,
                incompressible(8192),
            )],
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.len() > 6000,
            "the entry must be long enough to cut inside: {} bytes",
            bytes.len()
        );
        // Cut well inside the compressed stream, keeping the entry header intact.
        std::fs::write(&path, &bytes[..2048]).unwrap();

        let pack = PackFile::open(&path).unwrap();
        let error = pack.read_entry(built.offsets[0]).unwrap_err();
        assert_eq!(
            error.form(),
            form::PACK_TRUNCATED,
            "a pack cut mid-stream must be reported as truncated, not as corrupt deflate: {error}"
        );
    }

    #[test]
    fn an_entry_whose_declared_size_disagrees_with_its_stream_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        let built = testpack::build(
            &path,
            &[testpack::Entry::ObjectWithDeclaredSize(
                ObjectKind::Blob,
                b"hello".to_vec(),
                99,
            )],
        );
        let pack = PackFile::open(&path).unwrap();
        let error = pack.read_entry(built.offsets[0]).unwrap_err();
        assert_eq!(error.form(), form::PACK_ENTRY_SIZE_DISAGREES);
    }

    #[test]
    fn a_reserved_or_invalid_entry_type_is_refused() {
        for type_id in [0u8, 5] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.pack");
            let built = testpack::build(
                &path,
                &[testpack::Entry::RawType(type_id, b"hello".to_vec())],
            );
            let pack = PackFile::open(&path).unwrap();
            let error = pack.read_entry(built.offsets[0]).unwrap_err();
            assert_eq!(
                error.form(),
                form::PACK_ENTRY_UNKNOWN_TYPE,
                "type {type_id} was not refused"
            );
        }
    }

    #[test]
    fn an_entry_declaring_a_size_past_the_object_bound_is_refused_before_inflating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        let built = testpack::build(
            &path,
            &[testpack::Entry::ObjectWithDeclaredSize(
                ObjectKind::Blob,
                b"hello".to_vec(),
                MAX_OBJECT_BYTES as u64 + 1,
            )],
        );
        let pack = PackFile::open(&path).unwrap();
        assert_eq!(
            pack.read_entry(built.offsets[0]).unwrap_err().form(),
            form::OBJECT_TOO_LARGE
        );
    }

    #[test]
    fn an_ofs_delta_whose_base_offset_is_not_inside_the_pack_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        let built = testpack::build(
            &path,
            &[testpack::Entry::OfsDeltaRaw {
                backward: 1_000_000,
                delta: delta_of(0, 1, &[0x01, b'x']),
            }],
        );
        let pack = PackFile::open(&path).unwrap();
        assert_eq!(
            pack.read_entry(built.offsets[0]).unwrap_err().form(),
            form::OFS_DELTA_BAD_OFFSET
        );

        // A zero backward offset would name the delta itself, which is a cycle of length one.
        let path = dir.path().join("self.pack");
        let built = testpack::build(
            &path,
            &[testpack::Entry::OfsDeltaRaw {
                backward: 0,
                delta: delta_of(0, 1, &[0x01, b'x']),
            }],
        );
        let pack = PackFile::open(&path).unwrap();
        assert_eq!(
            pack.read_entry(built.offsets[0]).unwrap_err().form(),
            form::OFS_DELTA_BAD_OFFSET
        );
    }

    #[test]
    fn an_offset_inside_the_pack_header_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pack");
        testpack::build(
            &path,
            &[testpack::Entry::Object(ObjectKind::Blob, b"x".to_vec())],
        );
        let pack = PackFile::open(&path).unwrap();
        assert_eq!(
            pack.read_entry(0).unwrap_err().form(),
            form::OFS_DELTA_BAD_OFFSET
        );
    }

    #[test]
    fn a_file_too_short_to_be_a_pack_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.pack");
        std::fs::write(&path, b"PACK").unwrap();
        assert_eq!(
            PackFile::open(&path).unwrap_err().form(),
            form::PACK_TRUNCATED
        );
    }
}
