//! A 20-byte Git object id.
//!
//! SHA-1 only, and that is a decision rather than an omission: a SHA-256 repository is **detected
//! and refused** by [`crate::gitobj::ObjectStore::open`] rather than misread, because supporting
//! both would double the hash plumbing through every module for a format that is still
//! experimental, and the failure mode being avoided is producing 20-byte prefixes of real 32-byte
//! ids and treating them as identities.
//!
//! The derived [`Ord`] is byte-lexicographic, which is exactly the order a `.idx` v2 sorts its
//! object table in. That is what makes the binary search in [`crate::gitobj::packidx`] correct
//! rather than coincidentally working.

use std::fmt;

/// Bytes in an object id.
pub const OID_BYTES: usize = 20;

/// Hex characters in an object id's text form.
pub const OID_HEX_CHARS: usize = OID_BYTES * 2;

/// A Git object id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Oid([u8; OID_BYTES]);

impl Oid {
    /// Build an id from its raw bytes.
    pub const fn from_bytes(bytes: [u8; OID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Build an id from a slice, which must be exactly [`OID_BYTES`] long.
    ///
    /// `None` rather than a panic: the slice comes from a packfile, and a packfile is
    /// attacker-controlled input.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let array: [u8; OID_BYTES] = bytes.try_into().ok()?;
        Some(Self(array))
    }

    /// Parse the 40-character lowercase or uppercase hex form.
    ///
    /// `None` for anything else, including the 64-character SHA-256 form: a 64-hex string is a
    /// real object id in some repository, and silently truncating it to 40 characters would mint a
    /// plausible-looking identity for an object that does not exist.
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != OID_HEX_CHARS {
            return None;
        }
        let bytes = text.as_bytes();
        let mut out = [0u8; OID_BYTES];
        for (index, slot) in out.iter_mut().enumerate() {
            let high = hex_value(bytes[index * 2])?;
            let low = hex_value(bytes[index * 2 + 1])?;
            *slot = (high << 4) | low;
        }
        Some(Self(out))
    }

    /// The raw bytes.
    pub const fn as_bytes(&self) -> &[u8; OID_BYTES] {
        &self.0
    }

    /// The first byte, which is the `.idx` fanout table's key.
    pub const fn first_byte(&self) -> u8 {
        self.0[0]
    }

    /// The 40-character lowercase hex form.
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(OID_HEX_CHARS);
        for byte in self.0 {
            out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble is a hex digit"));
            out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble is a hex digit"));
        }
        out
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Oid({})", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_in_both_cases() {
        let text = "18156416b5128537cb41d292619c182151aa5dad";
        let oid = Oid::from_hex(text).expect("40 hex characters parse");
        assert_eq!(oid.to_hex(), text);
        assert_eq!(Oid::from_hex(&text.to_uppercase()), Some(oid));
        assert_eq!(oid.first_byte(), 0x18);
    }

    /// A 64-hex SHA-256 id must not be accepted and truncated: that would mint an identity.
    #[test]
    fn a_sha256_length_id_is_refused_rather_than_truncated() {
        let sha256 = "a".repeat(64);
        assert_eq!(Oid::from_hex(&sha256), None);
        assert_eq!(Oid::from_hex(&"a".repeat(39)), None);
        assert_eq!(Oid::from_hex(&"a".repeat(41)), None);
        assert_eq!(Oid::from_hex(""), None);
    }

    #[test]
    fn non_hex_is_refused() {
        assert_eq!(Oid::from_hex(&"g".repeat(40)), None);
        // 39 hex characters and a space is the shape a truncated ref file has.
        assert_eq!(Oid::from_hex(&format!("{} ", "a".repeat(39))), None);
    }

    #[test]
    fn from_slice_requires_exactly_twenty_bytes() {
        assert!(Oid::from_slice(&[0u8; 20]).is_some());
        assert!(Oid::from_slice(&[0u8; 19]).is_none());
        assert!(Oid::from_slice(&[0u8; 21]).is_none());
        assert!(Oid::from_slice(&[]).is_none());
    }

    /// The ordering the `.idx` binary search depends on.
    #[test]
    fn ordering_is_byte_lexicographic() {
        let low = Oid::from_bytes([0x00; 20]);
        let mut middle_bytes = [0x00; 20];
        middle_bytes[0] = 0x01;
        let middle = Oid::from_bytes(middle_bytes);
        let mut high_bytes = [0x00; 20];
        high_bytes[19] = 0x01;
        let high = Oid::from_bytes(high_bytes);
        assert!(low < high);
        assert!(high < middle, "the first byte must dominate the last");
    }
}
