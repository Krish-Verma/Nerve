//! `.idx` v2 — the fanout table, the sorted object ids, the CRCs, the offsets, and the 64-bit
//! offset overflow table.
//!
//! # Layout, and every field this reader checks
//!
//! ```text
//!   0   4   magic \377tOc
//!   4   4   version, big-endian; must be 2
//!   8  1024 fanout: 256 big-endian u32, cumulative counts by first oid byte
//!       20N sorted object ids
//!        4N CRC32 of each entry's compressed data
//!        4N offsets; if the high bit is set the low 31 bits index the table below
//!        8M 64-bit offsets, for packs larger than 2 GiB
//!        20 the pack's SHA-1
//!        20 this index's SHA-1
//! ```
//!
//! `N` comes from `fanout[255]`. `M` is **derived from the file's length**, which is why the length
//! check is exact rather than a lower bound: the only way to know how many 64-bit offsets an index
//! carries is that the remaining bytes divide evenly by eight. An index whose length does not work
//! out is refused rather than read as far as it goes.
//!
//! # Why the fanout must be non-decreasing
//!
//! It is a *cumulative* count, so `fanout[b]` is the number of objects whose first byte is `≤ b`.
//! The search range for first byte `b` is `fanout[b - 1] .. fanout[b]`. A decreasing entry makes
//! that range invalid — a reversed or wrapping range — and the binary search over it would be
//! answering a question about the wrong slice of the table. Clamping it would silently search the
//! wrong range; refusing states the finding.
//!
//! # `.idx` v1, and telling it apart from a file that is not an index
//!
//! A v1 index has no magic: it begins with the fanout table directly, followed by `N` records of a
//! 4-byte offset and a 20-byte id, then the two checksums. That is how Git distinguishes the two
//! versions — magic or no magic — and it was superseded in 2007.
//!
//! "No magic" alone is not enough to *report* a version, though: a truncated file, a text file, or a
//! v2 index whose first four bytes were overwritten also has no magic, and calling any of those
//! "version 1" would state something false. So [`looks_like_v1`] checks the v1 layout — a
//! non-decreasing fanout and a length that works out exactly for the count it declares — and only
//! then is the file reported as version 1, through
//! [`super::StoreLimits::unsupported_index_versions`]. Anything else is [`Error::IdxBadMagic`]: not
//! an index this reader recognises, rather than an old one.

use super::oid::{Oid, OID_BYTES};
use super::{Error, Result};

/// The four bytes a v2-or-later `.idx` begins with.
pub const IDX_MAGIC: [u8; 4] = [0xff, b't', b'O', b'c'];

/// The only `.idx` version this reader implements.
pub const SUPPORTED_IDX_VERSION: u32 = 2;

/// The version a magic-less `.idx` is reported as.
pub const LEGACY_IDX_VERSION: u32 = 1;

/// Bytes before the fanout table: magic and version.
const HEADER_BYTES: usize = 8;

/// Bytes in the fanout table: 256 big-endian `u32`.
const FANOUT_BYTES: usize = 256 * 4;

/// Bytes per object across the id, CRC and offset tables.
const PER_OBJECT_BYTES: usize = OID_BYTES + 4 + 4;

/// The two trailing SHA-1 checksums.
const TRAILER_BYTES: usize = 40;

/// A parsed `.idx` v2, holding the file's bytes and the offsets of its tables.
#[derive(Debug)]
pub struct PackIndex {
    bytes: Vec<u8>,
    count: usize,
    large_offset_count: usize,
}

impl PackIndex {
    /// Parse an index from the whole file's bytes.
    ///
    /// Every structural claim the file makes is checked here rather than at lookup time, so a
    /// lookup cannot read outside a table: the ranges were proved to exist when the index opened.
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        // The magic is checked before the length, and the order matters: a v1 index is *shorter*
        // than the v2 minimum, so checking the length first would report every v1 index as
        // truncated and the version would never be stated.
        if bytes.len() < 4 {
            return Err(Error::IdxTruncated(format!(
                "{} bytes is shorter than the magic",
                bytes.len()
            )));
        }
        if bytes[..4] != IDX_MAGIC {
            // A version is only *reported* when the v1 layout actually checks out. Calling every
            // magic-less file "version 1" would state something false about a truncated or
            // corrupted v2 index.
            return Err(if looks_like_v1(&bytes) {
                Error::IdxUnsupportedVersion(LEGACY_IDX_VERSION)
            } else {
                Error::IdxBadMagic
            });
        }
        if bytes.len() < HEADER_BYTES + FANOUT_BYTES + TRAILER_BYTES {
            return Err(Error::IdxTruncated(format!(
                "{} bytes is shorter than an empty v2 index",
                bytes.len()
            )));
        }
        let version = be_u32(&bytes, 4);
        if version != SUPPORTED_IDX_VERSION {
            return Err(Error::IdxUnsupportedVersion(version));
        }

        // The fanout is cumulative, so it must be non-decreasing across all 256 entries.
        let mut previous = 0u32;
        for bucket in 0..256 {
            let value = be_u32(&bytes, HEADER_BYTES + bucket * 4);
            if value < previous {
                return Err(Error::IdxFanoutNotMonotonic);
            }
            previous = value;
        }
        let count = previous as usize;

        // Exact length arithmetic, in `u64` so a hostile object count cannot overflow into a
        // plausible-looking total on a 32-bit target.
        let fixed = (HEADER_BYTES + FANOUT_BYTES + TRAILER_BYTES) as u64;
        let tables = (count as u64)
            .checked_mul(PER_OBJECT_BYTES as u64)
            .ok_or_else(|| Error::IdxTruncated(format!("object count {count} overflows")))?;
        let minimum = fixed
            .checked_add(tables)
            .ok_or_else(|| Error::IdxTruncated(format!("object count {count} overflows")))?;
        let length = bytes.len() as u64;
        if length < minimum {
            return Err(Error::IdxTruncated(format!(
                "{length} bytes for {count} objects, which needs at least {minimum}"
            )));
        }
        let remainder = length - minimum;
        if !remainder.is_multiple_of(8) {
            return Err(Error::IdxTruncated(format!(
                "{remainder} trailing bytes is not a whole number of 64-bit offsets"
            )));
        }
        let large_offset_count = (remainder / 8) as usize;

        Ok(Self {
            bytes,
            count,
            large_offset_count,
        })
    }

    /// How many objects this index describes.
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether the index describes no objects at all, which is legal but unusual.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// How many entries the 64-bit offset overflow table holds.
    pub const fn large_offset_count(&self) -> usize {
        self.large_offset_count
    }

    /// The object id at table position `position`.
    fn oid_at(&self, position: usize) -> Oid {
        let start = HEADER_BYTES + FANOUT_BYTES + position * OID_BYTES;
        Oid::from_slice(&self.bytes[start..start + OID_BYTES]).expect("a 20-byte slice")
    }

    /// The cumulative fanout count for first byte `bucket`.
    fn fanout(&self, bucket: usize) -> usize {
        be_u32(&self.bytes, HEADER_BYTES + bucket * 4) as usize
    }

    /// The table position of `oid`, found through the fanout rather than by scanning.
    ///
    /// The fanout narrows the search to the objects sharing the id's first byte before the binary
    /// search runs, which is the whole reason a `.idx` exists.
    pub fn find(&self, oid: &Oid) -> Option<usize> {
        let bucket = oid.first_byte() as usize;
        let low = if bucket == 0 {
            0
        } else {
            self.fanout(bucket - 1)
        };
        let high = self.fanout(bucket);
        if low > high || high > self.count {
            // Unreachable for an index that parsed, and asserted rather than assumed: `parse`
            // proved the fanout non-decreasing and `count == fanout[255]`.
            return None;
        }

        let mut lower = low;
        let mut upper = high;
        while lower < upper {
            let middle = lower + (upper - lower) / 2;
            match self.oid_at(middle).cmp(oid) {
                std::cmp::Ordering::Less => lower = middle + 1,
                std::cmp::Ordering::Greater => upper = middle,
                std::cmp::Ordering::Equal => return Some(middle),
            }
        }
        None
    }

    /// The pack offset of the object at table position `position`.
    pub fn offset_at(&self, position: usize) -> Result<u64> {
        let offsets_start = HEADER_BYTES + FANOUT_BYTES + self.count * (OID_BYTES + 4);
        let raw = be_u32(&self.bytes, offsets_start + position * 4);
        if raw & 0x8000_0000 == 0 {
            return Ok(u64::from(raw));
        }
        let index = raw & 0x7fff_ffff;
        if index as usize >= self.large_offset_count {
            return Err(Error::IdxLargeOffsetOutOfRange(index));
        }
        let large_start = offsets_start + self.count * 4;
        Ok(be_u64(&self.bytes, large_start + index as usize * 8))
    }

    /// The pack offset of `oid`, or `None` when this index does not describe it.
    pub fn offset_of(&self, oid: &Oid) -> Result<Option<u64>> {
        match self.find(oid) {
            Some(position) => self.offset_at(position).map(Some),
            None => Ok(None),
        }
    }

    /// The CRC32 the index recorded for the entry at `position`.
    ///
    /// Read and exposed, and **not** checked against the pack. Checking it would detect corruption
    /// `git fsck` exists for, and the entry's own declared-size check already catches the cases that
    /// would otherwise produce silently wrong bytes. Stated here because an unstated non-check is
    /// the kind of thing a later reader assumes was done.
    pub fn crc_at(&self, position: usize) -> u32 {
        let start = HEADER_BYTES + FANOUT_BYTES + self.count * OID_BYTES + position * 4;
        be_u32(&self.bytes, start)
    }

    /// Every object id in the index, in table order — which is ascending.
    pub fn oids(&self) -> impl Iterator<Item = Oid> + '_ {
        (0..self.count).map(|position| self.oid_at(position))
    }
}

/// Whether these bytes have the layout of a `.idx` **v1**.
///
/// A v1 index is `256 * u32` fanout, then `N` records of `u32` offset plus a 20-byte id, then two
/// 20-byte checksums — 24 bytes per object, and no magic. The length has to work out exactly for the
/// count the fanout declares, which is a strong enough check that a v2 index with its magic
/// overwritten does not accidentally satisfy it.
pub fn looks_like_v1(bytes: &[u8]) -> bool {
    const V1_PER_OBJECT_BYTES: u64 = 4 + OID_BYTES as u64;
    if bytes.len() < FANOUT_BYTES + TRAILER_BYTES {
        return false;
    }
    let mut previous = 0u32;
    for bucket in 0..256 {
        let value = be_u32(bytes, bucket * 4);
        if value < previous {
            return false;
        }
        previous = value;
    }
    let Some(records) = u64::from(previous).checked_mul(V1_PER_OBJECT_BYTES) else {
        return false;
    };
    (FANOUT_BYTES + TRAILER_BYTES) as u64 + records == bytes.len() as u64
}

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn be_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_be_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
        bytes[at + 4],
        bytes[at + 5],
        bytes[at + 6],
        bytes[at + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitobj::form;

    /// A well-formed `.idx` v2. One builder, shared with the pack and store tests.
    fn build_idx(entries: &[(Oid, u64)]) -> Vec<u8> {
        crate::gitobj::testpack::idx_bytes(entries)
    }

    fn oid(first: u8, last: u8) -> Oid {
        let mut bytes = [0u8; 20];
        bytes[0] = first;
        bytes[19] = last;
        Oid::from_bytes(bytes)
    }

    #[test]
    fn a_well_formed_index_finds_every_object_through_the_fanout() {
        let entries = vec![
            (oid(0x00, 1), 12),
            (oid(0x00, 2), 40),
            (oid(0x18, 1), 100),
            (oid(0xff, 9), 2000),
        ];
        let index = PackIndex::parse(build_idx(&entries)).expect("a well-formed index");
        assert_eq!(index.len(), 4);
        assert!(!index.is_empty());
        assert_eq!(index.large_offset_count(), 0);
        for (oid, offset) in &entries {
            assert_eq!(
                index.offset_of(oid).unwrap(),
                Some(*offset),
                "{oid} was not found"
            );
        }
        assert_eq!(index.offset_of(&oid(0x18, 2)).unwrap(), None);
        assert_eq!(index.oids().count(), 4);
        // The table is ascending, which is what makes the binary search valid.
        let collected: Vec<Oid> = index.oids().collect();
        let mut expected = collected.clone();
        expected.sort();
        assert_eq!(collected, expected);
    }

    #[test]
    fn an_empty_index_is_legal_and_finds_nothing() {
        let index = PackIndex::parse(build_idx(&[])).expect("an empty index parses");
        assert_eq!(index.len(), 0);
        assert!(index.is_empty());
        assert_eq!(index.offset_of(&oid(0x00, 1)).unwrap(), None);
    }

    /// A **real** v1 index is refused with its version stated, so a repository still carrying one is
    /// told which version it has rather than that its index is unreadable.
    #[test]
    fn a_real_version_one_index_is_reported_as_version_one() {
        for entries in [
            vec![],
            vec![(oid(1, 1), 12)],
            vec![(oid(0, 1), 12), (oid(0xff, 2), 40)],
        ] {
            let bytes = crate::gitobj::testpack::idx_v1_bytes(&entries);
            assert!(super::looks_like_v1(&bytes));
            match PackIndex::parse(bytes).unwrap_err() {
                Error::IdxUnsupportedVersion(version) => assert_eq!(version, LEGACY_IDX_VERSION),
                other => panic!("expected a stated version, got {other:?}"),
            }
        }
    }

    /// A v2 index whose magic was overwritten is **not** a v1 index, and saying so would be false.
    #[test]
    fn a_magic_less_file_that_is_not_a_v1_index_is_bad_magic_rather_than_version_one() {
        let mut bytes = build_idx(&[(oid(1, 1), 12)]);
        bytes[..4].copy_from_slice(&[0xff; 4]);
        assert!(!super::looks_like_v1(&bytes));
        assert_eq!(
            PackIndex::parse(bytes).unwrap_err().form(),
            form::IDX_BAD_MAGIC
        );

        // Neither is a text file, nor a short one.
        for junk in [
            &b"this is not a pack index at all, it is prose"[..],
            &[0u8; 3][..],
        ] {
            let error = PackIndex::parse(junk.to_vec()).unwrap_err();
            assert!(
                error.form() == form::IDX_BAD_MAGIC || error.form() == form::IDX_TRUNCATED,
                "{} was reported as {}",
                junk.len(),
                error.form()
            );
        }
    }

    #[test]
    fn an_unsupported_version_is_refused_with_the_version_stated() {
        for version in [0u32, 1, 3, 4, u32::MAX] {
            let mut bytes = build_idx(&[(oid(1, 1), 12)]);
            bytes[4..8].copy_from_slice(&version.to_be_bytes());
            match PackIndex::parse(bytes).unwrap_err() {
                Error::IdxUnsupportedVersion(reported) => assert_eq!(reported, version),
                other => panic!("expected a stated version for {version}, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_non_monotonic_fanout_is_refused_rather_than_clamped() {
        let entries = vec![(oid(0x00, 1), 12), (oid(0x80, 1), 40)];
        let mut bytes = build_idx(&entries);
        // Lower bucket 0x40 below bucket 0x00's count, which makes bucket 0x40's range reversed.
        bytes[HEADER_BYTES + 0x40 * 4..HEADER_BYTES + 0x40 * 4 + 4]
            .copy_from_slice(&0u32.to_be_bytes());
        bytes[HEADER_BYTES..HEADER_BYTES + 4].copy_from_slice(&5u32.to_be_bytes());
        let error = PackIndex::parse(bytes).unwrap_err();
        assert_eq!(error.form(), form::IDX_FANOUT_NOT_MONOTONIC);
    }

    #[test]
    fn a_truncated_index_is_refused() {
        let bytes = build_idx(&[(oid(1, 1), 12), (oid(2, 2), 40)]);
        for cut in [0usize, 8, 100, 1031, bytes.len() - 1] {
            let error = PackIndex::parse(bytes[..cut].to_vec()).unwrap_err();
            assert!(
                error.form() == form::IDX_TRUNCATED || error.form() == form::IDX_BAD_MAGIC,
                "cutting to {cut} gave {}",
                error.form()
            );
        }
    }

    /// The length must work out *exactly*, because that is the only way `M` is knowable.
    #[test]
    fn trailing_bytes_that_are_not_a_whole_number_of_large_offsets_are_refused() {
        let mut bytes = build_idx(&[(oid(1, 1), 12)]);
        bytes.push(0);
        let error = PackIndex::parse(bytes).unwrap_err();
        assert_eq!(error.form(), form::IDX_TRUNCATED);
    }

    #[test]
    fn a_declared_object_count_larger_than_the_file_is_refused() {
        let mut bytes = build_idx(&[(oid(1, 1), 12)]);
        // Claim 4 billion objects in a 1100-byte file.
        for bucket in 0..256 {
            bytes[HEADER_BYTES + bucket * 4..HEADER_BYTES + bucket * 4 + 4]
                .copy_from_slice(&0xffff_fff0u32.to_be_bytes());
        }
        let error = PackIndex::parse(bytes).unwrap_err();
        assert_eq!(error.form(), form::IDX_TRUNCATED);
    }

    /// A 64-bit offset entry indexes the overflow table, and an index past its end is refused
    /// rather than read out of the trailer.
    #[test]
    fn a_large_offset_index_past_the_table_is_refused() {
        let entries = vec![(oid(1, 1), 12)];
        let mut bytes = build_idx(&entries);
        let offsets_start = HEADER_BYTES + FANOUT_BYTES + entries.len() * (OID_BYTES + 4);
        bytes[offsets_start..offsets_start + 4].copy_from_slice(&0x8000_0000u32.to_be_bytes());
        let index = PackIndex::parse(bytes).expect("the structure is still valid");
        assert_eq!(index.large_offset_count(), 0);
        let error = index.offset_of(&oid(1, 1)).unwrap_err();
        assert_eq!(error.form(), form::IDX_LARGE_OFFSET_OUT_OF_RANGE);
    }

    #[test]
    fn a_large_offset_entry_inside_the_table_resolves_to_the_64_bit_value() {
        let entries = vec![(oid(1, 1), 12)];
        let mut bytes = build_idx(&entries);
        let offsets_start = HEADER_BYTES + FANOUT_BYTES + entries.len() * (OID_BYTES + 4);
        bytes[offsets_start..offsets_start + 4].copy_from_slice(&0x8000_0000u32.to_be_bytes());
        // Splice an 8-byte overflow table in before the two trailing checksums.
        let trailer_start = bytes.len() - TRAILER_BYTES;
        let mut spliced = bytes[..trailer_start].to_vec();
        spliced.extend_from_slice(&5_000_000_000u64.to_be_bytes());
        spliced.extend_from_slice(&bytes[trailer_start..]);

        let index = PackIndex::parse(spliced).expect("a valid index with one large offset");
        assert_eq!(index.large_offset_count(), 1);
        assert_eq!(index.offset_of(&oid(1, 1)).unwrap(), Some(5_000_000_000));
    }

    #[test]
    fn the_crc_table_is_readable_and_is_not_used_as_a_check() {
        let index = PackIndex::parse(build_idx(&[(oid(1, 1), 12)])).unwrap();
        assert_eq!(index.crc_at(0), 0, "the builder writes zero CRCs");
        // A zero CRC is not a refusal: nothing verifies it, by decision.
        assert_eq!(index.offset_of(&oid(1, 1)).unwrap(), Some(12));
    }
}
