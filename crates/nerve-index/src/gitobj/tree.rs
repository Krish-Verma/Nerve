//! A tree object's entries.
//!
//! ```text
//!   <mode, octal ASCII> <name> NUL <20 raw oid bytes>    repeated to the end
//! ```
//!
//! No count, no terminator, no padding: the object ends when the bytes do, so a truncated tree is
//! indistinguishable from a complete one except by the arithmetic not working out. That is why the
//! whole tree is refused when any entry is malformed rather than the entries before it being kept —
//! a prefix of a tree read as a tree is a claim that the remaining paths do not exist.
//!
//! # Names are bytes, and are carried verbatim
//!
//! A tree entry's name is not text: Git stores whatever the filesystem gave it. It is carried as
//! `Vec<u8>` and neither validated as UTF-8 nor normalised, the same rule [`crate::coverage`]
//! follows for an `SF:` path — path handling belongs to [`crate::discover::canonical_child`], and a
//! reader that cleaned a name here would hide the input the guard exists to refuse.
//!
//! What *is* refused is a name the **format** forbids: empty, or containing `/`, `.` or `..`. Those
//! are not paths Git will write — a `/` would make one entry claim to be a subtree — so a tree
//! carrying one is malformed rather than unusual.

use super::oid::{Oid, OID_BYTES};
use super::{Error, Result};

/// One entry in a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// The mode, as the octal digits state it: `100644`, `100755`, `120000`, `40000`, `160000`.
    ///
    /// Kept as the parsed number rather than as a `FileKind`, because classifying it is a decision
    /// Slice 12b makes and a reader that classified it would be asserting something.
    pub mode: u32,
    /// The name, exactly as Git stored it. Bytes, not text.
    pub name: Vec<u8>,
    /// The object this entry names.
    pub oid: Oid,
}

impl TreeEntry {
    /// Whether the mode marks a subtree (`040000`).
    pub const fn is_tree(&self) -> bool {
        self.mode == 0o040000
    }

    /// Whether the mode marks a gitlink — a submodule commit (`160000`).
    ///
    /// Worth its own question because the object it names is a **commit** that this repository does
    /// not contain, so reading it will legitimately produce `Ok(None)`.
    pub const fn is_gitlink(&self) -> bool {
        self.mode == 0o160000
    }
}

/// Parse a tree object's content bytes.
pub fn parse_tree(bytes: &[u8]) -> Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let space = bytes[cursor..]
            .iter()
            .position(|byte| *byte == b' ')
            .map(|at| cursor + at)
            .ok_or_else(|| Error::TreeEntryMalformed("no space after the mode".to_string()))?;
        let mode = parse_mode(&bytes[cursor..space])?;

        let nul = bytes[space + 1..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|at| space + 1 + at)
            .ok_or_else(|| Error::TreeEntryMalformed("no NUL after the name".to_string()))?;
        let name = &bytes[space + 1..nul];
        check_name(name)?;

        let oid_start = nul + 1;
        let oid_end = oid_start
            .checked_add(OID_BYTES)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| {
                Error::TreeEntryMalformed("entry ends before its object id".to_string())
            })?;
        let oid = Oid::from_slice(&bytes[oid_start..oid_end]).expect("a 20-byte slice");

        entries.push(TreeEntry {
            mode,
            name: name.to_vec(),
            oid,
        });
        cursor = oid_end;
    }
    Ok(entries)
}

/// Parse the octal mode. One to six digits, `0`–`7`, and no leading zero except for the digit `0`.
///
/// Git writes `40000` rather than `040000`, so a leading zero is a sign the bytes were produced by
/// something else and is refused rather than accepted as equivalent.
fn parse_mode(bytes: &[u8]) -> Result<u32> {
    if bytes.is_empty() || bytes.len() > 6 {
        return Err(Error::TreeEntryMalformed(format!(
            "mode is {} digits",
            bytes.len()
        )));
    }
    if bytes[0] == b'0' && bytes.len() > 1 {
        return Err(Error::TreeEntryMalformed(
            "mode has a leading zero".to_string(),
        ));
    }
    let mut mode = 0u32;
    for byte in bytes {
        if !(b'0'..=b'7').contains(byte) {
            return Err(Error::TreeEntryMalformed("mode is not octal".to_string()));
        }
        mode = mode * 8 + u32::from(byte - b'0');
    }
    Ok(mode)
}

fn check_name(name: &[u8]) -> Result<()> {
    if name.is_empty() {
        return Err(Error::TreeEntryMalformed("empty name".to_string()));
    }
    if name.contains(&b'/') {
        return Err(Error::TreeEntryMalformed(
            "a name containing '/' would claim to be a subtree".to_string(),
        ));
    }
    if name == b"." || name == b".." {
        return Err(Error::TreeEntryMalformed(
            "'.' and '..' are not tree entry names".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitobj::form;

    fn entry(mode: &str, name: &[u8], last: u8) -> Vec<u8> {
        let mut out = mode.as_bytes().to_vec();
        out.push(b' ');
        out.extend_from_slice(name);
        out.push(0);
        let mut oid = [0u8; OID_BYTES];
        oid[OID_BYTES - 1] = last;
        out.extend_from_slice(&oid);
        out
    }

    #[test]
    fn every_mode_git_writes_parses() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&entry("100644", b"README.md", 1));
        bytes.extend_from_slice(&entry("100755", b"run.sh", 2));
        bytes.extend_from_slice(&entry("120000", b"link", 3));
        bytes.extend_from_slice(&entry("40000", b"src", 4));
        bytes.extend_from_slice(&entry("160000", b"vendor", 5));

        let entries = parse_tree(&bytes).unwrap();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].mode, 0o100644);
        assert_eq!(entries[0].name, b"README.md");
        assert_eq!(entries[1].mode, 0o100755);
        assert_eq!(entries[2].mode, 0o120000);
        assert_eq!(entries[3].mode, 0o040000);
        assert!(entries[3].is_tree());
        assert!(!entries[3].is_gitlink());
        assert_eq!(entries[4].mode, 0o160000);
        assert!(entries[4].is_gitlink());
        assert!(!entries[4].is_tree());
    }

    #[test]
    fn an_empty_tree_is_legal() {
        assert_eq!(parse_tree(b"").unwrap(), Vec::new());
    }

    /// A name that is not UTF-8 is real and is carried, not refused and not replaced.
    #[test]
    fn a_non_utf8_name_is_carried_verbatim() {
        let entries = parse_tree(&entry("100644", &[0xff, 0xfe, 0x80], 1)).unwrap();
        assert_eq!(entries[0].name, vec![0xff, 0xfe, 0x80]);
    }

    /// A name that merely *looks* dangerous is content, and content is carried through to the guard
    /// rather than cleaned up here — the same rule the coverage reader follows for `SF:`.
    #[test]
    fn a_name_with_a_control_byte_or_a_backslash_is_carried_not_cleaned() {
        for name in [&b"a\x1fb"[..], &b"..\\..\\etc"[..], &b"-rf"[..], &b" "[..]] {
            let entries = parse_tree(&entry("100644", name, 1)).unwrap();
            assert_eq!(entries[0].name, name.to_vec(), "{name:?} was rewritten");
        }
    }

    #[test]
    fn a_name_the_format_forbids_is_refused() {
        for name in [&b""[..], &b"a/b"[..], &b"."[..], &b".."[..]] {
            assert_eq!(
                parse_tree(&entry("100644", name, 1)).unwrap_err().form(),
                form::TREE_ENTRY_MALFORMED,
                "{name:?} was not refused"
            );
        }
    }

    #[test]
    fn a_malformed_mode_is_refused() {
        for mode in ["", "0100644", "1006448", "8", "abc", "1234567"] {
            let mut bytes = mode.as_bytes().to_vec();
            bytes.push(b' ');
            bytes.extend_from_slice(b"name");
            bytes.push(0);
            bytes.extend_from_slice(&[0u8; OID_BYTES]);
            assert_eq!(
                parse_tree(&bytes).unwrap_err().form(),
                form::TREE_ENTRY_MALFORMED,
                "mode {mode:?} was not refused"
            );
        }
    }

    /// A tree cut short is refused **whole**. Keeping the entries before the cut would assert that
    /// the paths after it do not exist.
    #[test]
    fn a_truncated_tree_is_refused_whole_rather_than_partly_believed() {
        let mut bytes = entry("100644", b"a.txt", 1);
        bytes.extend_from_slice(&entry("100644", b"b.txt", 2));
        for cut in [1usize, 8, 20, bytes.len() - 1] {
            let error = parse_tree(&bytes[..cut]).unwrap_err();
            assert_eq!(
                error.form(),
                form::TREE_ENTRY_MALFORMED,
                "cutting to {cut} was not refused"
            );
        }
    }

    #[test]
    fn an_entry_with_no_nul_or_no_space_is_refused() {
        assert_eq!(
            parse_tree(b"100644README.md\0aaaaaaaaaaaaaaaaaaaa")
                .unwrap_err()
                .form(),
            form::TREE_ENTRY_MALFORMED
        );
        assert_eq!(
            parse_tree(b"100644 README.md").unwrap_err().form(),
            form::TREE_ENTRY_MALFORMED
        );
    }
}
