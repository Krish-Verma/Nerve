//! Loose objects: `objects/ab/cdef…`.
//!
//! A loose object file is one zlib stream whose inflated content is
//! `"<type> <size>\0<content>"`. Both halves of that header are checked against the stream, because
//! the format states the size twice — once in the header and once implicitly as the stream's length
//! — and a disagreement means one of them is lying with no way to tell which.
//!
//! **A disagreement is refused rather than resolved.** Trusting the header would hand a caller a
//! truncated object described as complete; trusting the stream would silently accept a file whose
//! own header contradicts it. There is no third value that is both safe and useful, so the object is
//! refused with both numbers stated. This is also the check that stands in for the SHA-1
//! verification this module deliberately does not do (see [`super`]): it catches the corruption that
//! would otherwise produce silently wrong bytes.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use super::inflate::{inflate_bounded, MAX_OBJECT_BYTES};
use super::oid::Oid;
use super::{Error, Object, ObjectKind, Result};

/// Bytes a loose object's `"<type> <size>\0"` header may occupy.
///
/// Derived rather than guessed: the longest type word is `commit` (6), plus a space, plus a decimal
/// `u64` at most 20 digits, plus the NUL, is 28. 64 is that with room to spare and small enough that
/// a header search never scans far.
pub const MAX_LOOSE_HEADER_BYTES: usize = 64;

/// The path a loose object with `oid` would occupy under `objects_dir`.
///
/// Built from the id's own hex, so there is no attacker-supplied path component: the two-character
/// directory and the 38-character file name are both derived from 20 bytes this reader produced.
pub fn loose_path(objects_dir: &Path, oid: &Oid) -> PathBuf {
    let hex = oid.to_hex();
    objects_dir.join(&hex[..2]).join(&hex[2..])
}

/// Read and parse the loose object at `path`.
///
/// `Ok(None)` when the file does not exist, which is the ordinary case: most objects in a real
/// repository are packed, and a partial clone is missing some entirely. Every other failure is a
/// refusal with a stated reason.
pub fn read_loose(path: &Path) -> Result<Option<Object>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::Io(err)),
    };
    // The bound covers header *and* content, so the slack is added on top of the object bound
    // rather than taken out of it. The content length is then checked against the object bound
    // directly, so the slack cannot be used to smuggle 64 extra bytes of object.
    let inflated = inflate_bounded(
        BufReader::new(file),
        MAX_OBJECT_BYTES + MAX_LOOSE_HEADER_BYTES,
    )?;
    parse_loose(&inflated).map(Some)
}

/// Parse an inflated loose object: `"<type> <size>\0<content>"`.
pub fn parse_loose(inflated: &[u8]) -> Result<Object> {
    let search = &inflated[..inflated.len().min(MAX_LOOSE_HEADER_BYTES)];
    let nul = search
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::LooseHeaderMalformed)?;
    let header = &inflated[..nul];
    let content = &inflated[nul + 1..];

    let space = header
        .iter()
        .position(|byte| *byte == b' ')
        .ok_or(Error::LooseHeaderMalformed)?;
    let kind = ObjectKind::from_word(&header[..space]).ok_or(Error::LooseUnknownType)?;

    let declared = parse_decimal(&header[space + 1..]).ok_or(Error::LooseHeaderMalformed)?;
    if declared != content.len() as u64 {
        return Err(Error::LooseDeclaredSizeDisagrees {
            declared,
            actual: content.len(),
        });
    }
    if content.len() > MAX_OBJECT_BYTES {
        return Err(Error::ObjectTooLarge {
            limit: MAX_OBJECT_BYTES,
            at_least: content.len(),
        });
    }

    Ok(Object::new(kind, content.to_vec()))
}

/// Parse the header's size field: ASCII decimal digits, nothing else.
///
/// No sign, no whitespace, no leading `+`, no hex, and no empty string. Git writes a plain decimal
/// and `u64::from_str` would accept forms Git never produces, so the digits are checked here rather
/// than delegated.
fn parse_decimal(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut value: u64 = 0;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).expect("writing to a Vec");
        encoder.finish().expect("finishing a Vec encoder")
    }

    fn loose_bytes(kind: &str, content: &[u8]) -> Vec<u8> {
        let mut raw = format!("{kind} {}\0", content.len()).into_bytes();
        raw.extend_from_slice(content);
        deflate(&raw)
    }

    #[test]
    fn all_four_types_parse() {
        for kind in ObjectKind::ALL {
            let mut raw = format!("{} 5\0", kind.as_str()).into_bytes();
            raw.extend_from_slice(b"hello");
            let object = parse_loose(&raw).expect("a well-formed header parses");
            assert_eq!(object.kind(), kind);
            assert_eq!(object.data(), b"hello");
        }
    }

    #[test]
    fn a_zero_length_object_is_legal() {
        let object = parse_loose(b"blob 0\0").expect("an empty blob is a real object");
        assert_eq!(object.kind(), ObjectKind::Blob);
        assert_eq!(object.data(), b"");
    }

    /// The load-bearing refusal of this module.
    #[test]
    fn a_declared_size_that_disagrees_with_the_stream_is_refused_not_resolved() {
        // Header claims more than the stream holds.
        let error = parse_loose(b"blob 99\0short").unwrap_err();
        match error {
            Error::LooseDeclaredSizeDisagrees { declared, actual } => {
                assert_eq!(declared, 99);
                assert_eq!(actual, 5);
            }
            other => panic!("expected a size disagreement, got {other:?}"),
        }

        // Header claims less than the stream holds. Equally refused: neither value wins.
        let error = parse_loose(b"blob 2\0abcdef").unwrap_err();
        assert_eq!(
            error.form(),
            super::super::form::LOOSE_DECLARED_SIZE_DISAGREES
        );

        // And an absurd claim is a disagreement rather than a size-bound refusal, because the
        // stream is what it is: reporting `object-too-large` here would name the wrong finding.
        let error = parse_loose(b"blob 18446744073709551615\0abc").unwrap_err();
        assert_eq!(
            error.form(),
            super::super::form::LOOSE_DECLARED_SIZE_DISAGREES
        );
    }

    #[test]
    fn a_header_with_no_nul_is_refused() {
        let error = parse_loose(b"blob 5").unwrap_err();
        assert_eq!(error.form(), super::super::form::LOOSE_HEADER_MALFORMED);
        // A NUL past the header window is not a header terminator.
        let mut far = b"blob ".to_vec();
        far.extend_from_slice(&[b'0'; MAX_LOOSE_HEADER_BYTES]);
        far.push(0);
        assert_eq!(
            parse_loose(&far).unwrap_err().form(),
            super::super::form::LOOSE_HEADER_MALFORMED
        );
    }

    #[test]
    fn a_header_with_no_space_is_refused() {
        assert_eq!(
            parse_loose(b"blob5\0hello").unwrap_err().form(),
            super::super::form::LOOSE_HEADER_MALFORMED
        );
    }

    #[test]
    fn an_unknown_type_word_is_refused_with_its_own_reason() {
        assert_eq!(
            parse_loose(b"commitish 5\0hello").unwrap_err().form(),
            super::super::form::LOOSE_UNKNOWN_TYPE
        );
        assert_eq!(
            parse_loose(b" 5\0hello").unwrap_err().form(),
            super::super::form::LOOSE_UNKNOWN_TYPE
        );
    }

    #[test]
    fn a_size_that_is_not_plain_decimal_is_refused() {
        for header in [
            &b"blob +5\0hello"[..],
            &b"blob -5\0hello"[..],
            &b"blob 0x5\0hello"[..],
            &b"blob 5 \0hello"[..],
            &b"blob \0hello"[..],
            // Past u64, so `checked_mul` refuses rather than wrapping.
            &b"blob 99999999999999999999999\0hello"[..],
        ] {
            assert_eq!(
                parse_loose(header).unwrap_err().form(),
                super::super::form::LOOSE_HEADER_MALFORMED,
                "{:?} was not refused",
                String::from_utf8_lossy(header)
            );
        }
    }

    #[test]
    fn an_absent_file_is_ok_none_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("objects").join("ab").join("cdef");
        assert_eq!(read_loose(&missing).unwrap(), None);
    }

    #[test]
    fn a_real_file_round_trips_through_the_reader() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("object");
        std::fs::write(&path, loose_bytes("commit", b"tree abc\n")).unwrap();
        let object = read_loose(&path).unwrap().expect("the file exists");
        assert_eq!(object, Object::Commit(b"tree abc\n".to_vec()));
    }

    /// A bomb in a *loose* object takes the same path as one in a pack entry, so the refusal must
    /// come from the inflate bound rather than from the header check.
    #[test]
    fn a_loose_bomb_is_refused_by_the_inflate_bound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("object");
        // Declares a small size, then carries far more than the bound. The header is never even
        // reached: the inflate refuses first.
        let mut raw = b"blob 4\0".to_vec();
        raw.extend_from_slice(&vec![0u8; MAX_OBJECT_BYTES + MAX_LOOSE_HEADER_BYTES + 1]);
        std::fs::write(&path, deflate(&raw)).unwrap();
        assert_eq!(
            read_loose(&path).unwrap_err().form(),
            super::super::form::OBJECT_TOO_LARGE
        );
    }

    #[test]
    fn the_object_path_is_built_from_the_id_alone() {
        let oid = Oid::from_hex("18156416b5128537cb41d292619c182151aa5dad").unwrap();
        let path = loose_path(Path::new("/repo/.git/objects"), &oid);
        assert!(path.ends_with("18/156416b5128537cb41d292619c182151aa5dad"));
    }
}
