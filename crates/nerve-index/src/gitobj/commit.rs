//! A commit object's fields.
//!
//! ```text
//!   tree <40 hex>
//!   parent <40 hex>          zero or more
//!   author Name <email> <unix seconds> <timezone>
//!   committer Name <email> <unix seconds> <timezone>
//!   …further headers, each optionally continued on lines beginning with a space
//!   <blank line>
//!   message
//! ```
//!
//! A plain struct, not an entity: Slice 12a produces no graph record. [`Commit`] is what Slice 12b
//! will build one *from*.
//!
//! # What is carried verbatim, and why
//!
//! An author's name and email are bytes, not text. A commit may carry an `encoding` header naming
//! something other than UTF-8, and a repository is attacker-controlled content (THREAT-MODEL A1), so
//! these are `Vec<u8>` and are neither validated as UTF-8 nor cleaned of control bytes — the same
//! rule [`crate::coverage`] follows for an `SF:` path and for the same reason: a refusal the guard
//! never sees is a refusal nobody reports. The timezone is likewise carried verbatim, because
//! historical repositories contain offsets no current rule would accept and refusing a real commit
//! would be worse than reporting an odd string.
//!
//! # What is checked
//!
//! Header **order and presence**, because those are what the format guarantees and what a caller
//! would otherwise have to guess at: `tree` first, then any parents, then `author`, then
//! `committer`. A commit that does not have that shape is refused with a stated reason rather than
//! parsed as far as it goes — a commit missing its `tree` is not a commit with an empty tree.

use super::oid::{Oid, OID_HEX_CHARS};
use super::{Error, Result};

/// Parents a single commit may name.
///
/// Bounded at 1024. An octopus merge is legal and the largest in the wild is on the order of 60
/// parents, so this admits every real commit while bounding what one hostile object can make Slice
/// 12b allocate.
pub const MAX_COMMIT_PARENTS: usize = 1024;

/// An author or committer line: who, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The name, exactly as the commit spelled it. Bytes, not text.
    pub name: Vec<u8>,
    /// The email, exactly as the commit spelled it, without the angle brackets.
    pub email: Vec<u8>,
    /// Seconds since the Unix epoch. Signed, because broken repositories contain negative values.
    pub timestamp: i64,
    /// The timezone field, verbatim — usually `+0000`, but not always.
    pub timezone: String,
}

/// A commit object's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The tree this commit names.
    pub tree: Oid,
    /// The parents, in the order the commit listed them. Empty for a root commit.
    pub parents: Vec<Oid>,
    /// Who wrote the change.
    pub author: Identity,
    /// Who committed it. Different from the author for a cherry-pick, a rebase or a patch applied
    /// on someone's behalf, which is why both are kept.
    pub committer: Identity,
    /// Everything after the blank line, verbatim, including any trailing newline.
    pub message: Vec<u8>,
}

/// Parse a commit object's content bytes — the payload, without the loose-object header.
pub fn parse_commit(bytes: &[u8]) -> Result<Commit> {
    let mut tree: Option<Oid> = None;
    let mut parents: Vec<Oid> = Vec::new();
    let mut author: Option<Identity> = None;
    let mut committer: Option<Identity> = None;

    let mut cursor = 0usize;
    let mut message_start = bytes.len();
    while cursor < bytes.len() {
        let end = bytes[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|at| cursor + at)
            .unwrap_or(bytes.len());
        let line = &bytes[cursor..end];
        let next = if end < bytes.len() {
            end + 1
        } else {
            bytes.len()
        };

        if line.is_empty() {
            // The blank line that separates headers from the message.
            message_start = next;
            break;
        }
        if line[0] == b' ' {
            // A continuation of the previous header — `gpgsig` is the common case. Recognised so
            // that its content is never mistaken for a header of its own.
            cursor = next;
            continue;
        }

        let space = line
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or_else(|| Error::CommitHeaderMalformed("header with no value".to_string()))?;
        let (key, value) = (&line[..space], &line[space + 1..]);

        match key {
            b"tree" => {
                if tree.is_some() || !parents.is_empty() || author.is_some() {
                    return Err(Error::CommitHeaderMalformed(
                        "tree must appear once and first".to_string(),
                    ));
                }
                tree = Some(parse_oid_header(value, "tree")?);
            }
            b"parent" => {
                if tree.is_none() || author.is_some() {
                    return Err(Error::CommitHeaderMalformed(
                        "parent must follow tree and precede author".to_string(),
                    ));
                }
                if parents.len() >= MAX_COMMIT_PARENTS {
                    return Err(Error::CommitParentsExceeded);
                }
                parents.push(parse_oid_header(value, "parent")?);
            }
            b"author" => {
                if tree.is_none() || author.is_some() {
                    return Err(Error::CommitHeaderMalformed(
                        "author must appear once, after tree".to_string(),
                    ));
                }
                author = Some(parse_identity(value)?);
            }
            b"committer" => {
                if author.is_none() || committer.is_some() {
                    return Err(Error::CommitHeaderMalformed(
                        "committer must appear once, after author".to_string(),
                    ));
                }
                committer = Some(parse_identity(value)?);
            }
            // `encoding`, `gpgsig`, `mergetag`, `HEAD` and anything a future Git adds. Recognised as
            // headers and not interpreted: interpreting one would be asserting something about it.
            _ => {}
        }
        cursor = next;
    }

    let tree = tree.ok_or_else(|| Error::CommitHeaderMalformed("no tree header".to_string()))?;
    let author =
        author.ok_or_else(|| Error::CommitHeaderMalformed("no author header".to_string()))?;
    let committer =
        committer.ok_or_else(|| Error::CommitHeaderMalformed("no committer header".to_string()))?;

    Ok(Commit {
        tree,
        parents,
        author,
        committer,
        message: bytes[message_start..].to_vec(),
    })
}

fn parse_oid_header(value: &[u8], field: &str) -> Result<Oid> {
    if value.len() != OID_HEX_CHARS {
        return Err(Error::CommitHeaderMalformed(format!(
            "{field} is {} characters, not {OID_HEX_CHARS}",
            value.len()
        )));
    }
    let text = std::str::from_utf8(value)
        .map_err(|_| Error::CommitHeaderMalformed(format!("{field} is not ASCII hex")))?;
    Oid::from_hex(text)
        .ok_or_else(|| Error::CommitHeaderMalformed(format!("{field} is not a hex object id")))
}

/// Parse `Name <email> <seconds> <timezone>`.
///
/// The email is delimited by the **last** `<`…`>` pair, because a name may legitimately contain a
/// `<` that Git did not sanitise, and taking the first would split the name.
fn parse_identity(value: &[u8]) -> Result<Identity> {
    let close = value
        .iter()
        .rposition(|byte| *byte == b'>')
        .ok_or_else(|| Error::CommitHeaderMalformed("identity has no '>'".to_string()))?;
    let open = value[..close]
        .iter()
        .rposition(|byte| *byte == b'<')
        .ok_or_else(|| Error::CommitHeaderMalformed("identity has no '<'".to_string()))?;

    let name = value[..open]
        .strip_suffix(b" ")
        .unwrap_or(&value[..open])
        .to_vec();
    let email = value[open + 1..close].to_vec();

    let rest = value[close + 1..]
        .strip_prefix(b" ")
        .ok_or_else(|| Error::CommitHeaderMalformed("no time after the email".to_string()))?;
    let mut fields = rest.splitn(2, |byte| *byte == b' ');
    let seconds = fields
        .next()
        .ok_or_else(|| Error::CommitHeaderMalformed("no timestamp".to_string()))?;
    let timezone = fields
        .next()
        .ok_or_else(|| Error::CommitHeaderMalformed("no timezone".to_string()))?;

    let seconds = std::str::from_utf8(seconds)
        .ok()
        .and_then(|text| text.parse::<i64>().ok())
        .ok_or_else(|| Error::CommitHeaderMalformed("timestamp is not an integer".to_string()))?;
    if timezone.is_empty() || timezone.contains(&b' ') {
        return Err(Error::CommitHeaderMalformed(
            "timezone is empty or has a space in it".to_string(),
        ));
    }
    // Verbatim, and lossy only for the timezone, which is ASCII in every form Git writes.
    let timezone = String::from_utf8_lossy(timezone).into_owned();

    Ok(Identity {
        name,
        email,
        timestamp: seconds,
        timezone,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitobj::form;

    const TREE: &str = "1baffc7c813737b1abcec5b04c32c35d06e75f13";
    const PARENT: &str = "25877452a998118d92916add890582e083b30180";

    fn commit_bytes(extra_headers: &str, parents: &[&str]) -> Vec<u8> {
        let mut text = format!("tree {TREE}\n");
        for parent in parents {
            text.push_str(&format!("parent {parent}\n"));
        }
        text.push_str("author Nerve Fixture <fixture@nerve.invalid> 1767225600 +0000\n");
        text.push_str("committer Nerve Fixture <fixture@nerve.invalid> 1767225601 -0800\n");
        text.push_str(extra_headers);
        text.push_str("\nthe message\n");
        text.into_bytes()
    }

    #[test]
    fn a_root_commit_parses() {
        let commit = parse_commit(&commit_bytes("", &[])).unwrap();
        assert_eq!(commit.tree, Oid::from_hex(TREE).unwrap());
        assert!(commit.parents.is_empty());
        assert_eq!(commit.author.name, b"Nerve Fixture");
        assert_eq!(commit.author.email, b"fixture@nerve.invalid");
        assert_eq!(commit.author.timestamp, 1_767_225_600);
        assert_eq!(commit.author.timezone, "+0000");
        assert_eq!(commit.committer.timestamp, 1_767_225_601);
        assert_eq!(commit.committer.timezone, "-0800");
        assert_eq!(commit.message, b"the message\n");
    }

    #[test]
    fn parents_are_kept_in_order() {
        let second = "2bc724850eeb19dc605a5fb97a182fdaf20232a8";
        let commit = parse_commit(&commit_bytes("", &[PARENT, second])).unwrap();
        assert_eq!(
            commit.parents,
            vec![
                Oid::from_hex(PARENT).unwrap(),
                Oid::from_hex(second).unwrap()
            ]
        );
    }

    /// A `gpgsig` header's continuation lines must not be read as headers of their own.
    #[test]
    fn a_multi_line_header_is_recognised_and_not_interpreted() {
        let signed = "gpgsig -----BEGIN PGP SIGNATURE-----\n \n iQIzBAABCAAd\n \
                      -----END PGP SIGNATURE-----\n";
        let commit = parse_commit(&commit_bytes(signed, &[PARENT])).unwrap();
        assert_eq!(commit.parents.len(), 1);
        assert_eq!(commit.message, b"the message\n");
    }

    #[test]
    fn an_unknown_header_is_ignored_rather_than_refused() {
        let commit =
            parse_commit(&commit_bytes("encoding ISO-8859-1\nsomething new\n", &[])).unwrap();
        assert_eq!(commit.message, b"the message\n");
    }

    #[test]
    fn a_commit_with_no_message_is_legal() {
        let mut text = format!("tree {TREE}\n");
        text.push_str("author A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\n");
        let commit = parse_commit(text.as_bytes()).unwrap();
        assert_eq!(commit.message, b"");
    }

    #[test]
    fn a_missing_tree_author_or_committer_is_refused() {
        let no_tree = "author A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nm\n";
        assert_eq!(
            parse_commit(no_tree.as_bytes()).unwrap_err().form(),
            form::COMMIT_HEADER_MALFORMED
        );
        let no_author = format!("tree {TREE}\ncommitter A <a@b> 1 +0000\n\nm\n");
        assert_eq!(
            parse_commit(no_author.as_bytes()).unwrap_err().form(),
            form::COMMIT_HEADER_MALFORMED
        );
        let no_committer = format!("tree {TREE}\nauthor A <a@b> 1 +0000\n\nm\n");
        assert_eq!(
            parse_commit(no_committer.as_bytes()).unwrap_err().form(),
            form::COMMIT_HEADER_MALFORMED
        );
    }

    #[test]
    fn headers_out_of_order_or_repeated_are_refused() {
        let parent_first = format!("parent {PARENT}\ntree {TREE}\n\nm\n");
        assert_eq!(
            parse_commit(parent_first.as_bytes()).unwrap_err().form(),
            form::COMMIT_HEADER_MALFORMED
        );
        let two_trees = format!("tree {TREE}\ntree {TREE}\n\nm\n");
        assert_eq!(
            parse_commit(two_trees.as_bytes()).unwrap_err().form(),
            form::COMMIT_HEADER_MALFORMED
        );
        let parent_after_author = format!(
            "tree {TREE}\nauthor A <a@b> 1 +0000\nparent {PARENT}\ncommitter A <a@b> 1 +0000\n\nm\n"
        );
        assert_eq!(
            parse_commit(parent_after_author.as_bytes())
                .unwrap_err()
                .form(),
            form::COMMIT_HEADER_MALFORMED
        );
    }

    #[test]
    fn a_tree_or_parent_that_is_not_a_hex_id_is_refused() {
        for value in ["", "not-hex", &"a".repeat(64), &"g".repeat(40)] {
            let text =
                format!("tree {value}\nauthor A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nm\n");
            assert_eq!(
                parse_commit(text.as_bytes()).unwrap_err().form(),
                form::COMMIT_HEADER_MALFORMED,
                "tree {value:?} was not refused"
            );
        }
    }

    #[test]
    fn a_malformed_identity_is_refused() {
        for identity in [
            "no angle brackets 1 +0000",
            "A <a@b>",
            "A <a@b> notanumber +0000",
            "A <a@b> 1",
            "A a@b> 1 +0000",
        ] {
            let text = format!("tree {TREE}\nauthor {identity}\ncommitter A <a@b> 1 +0000\n\nm\n");
            assert_eq!(
                parse_commit(text.as_bytes()).unwrap_err().form(),
                form::COMMIT_HEADER_MALFORMED,
                "{identity:?} was not refused"
            );
        }
    }

    /// A name containing an angle bracket is real, and the *last* pair delimits the email.
    #[test]
    fn a_name_containing_an_angle_bracket_does_not_split_the_name() {
        let text = format!(
            "tree {TREE}\nauthor A <weird> B <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nm\n"
        );
        let commit = parse_commit(text.as_bytes()).unwrap();
        assert_eq!(commit.author.name, b"A <weird> B");
        assert_eq!(commit.author.email, b"a@b");
    }

    /// Bytes, not text: a name that is not UTF-8 is carried rather than refused or replaced.
    #[test]
    fn a_non_utf8_name_is_carried_verbatim() {
        let mut bytes = format!("tree {TREE}\nauthor ").into_bytes();
        bytes.extend_from_slice(&[0xff, 0xfe]);
        bytes.extend_from_slice(b" <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nm\n");
        let commit = parse_commit(&bytes).unwrap();
        assert_eq!(commit.author.name, vec![0xff, 0xfe]);
    }

    #[test]
    fn a_negative_timestamp_is_carried_rather_than_refused() {
        let text =
            format!("tree {TREE}\nauthor A <a@b> -100 +0000\ncommitter A <a@b> 1 +0000\n\nm\n");
        let commit = parse_commit(text.as_bytes()).unwrap();
        assert_eq!(commit.author.timestamp, -100);
    }

    #[test]
    fn a_commit_with_too_many_parents_is_refused_at_the_bound() {
        let parents: Vec<String> = (0..=MAX_COMMIT_PARENTS)
            .map(|index| format!("{:040x}", index + 1))
            .collect();
        let mut text = format!("tree {TREE}\n");
        for parent in &parents {
            text.push_str(&format!("parent {parent}\n"));
        }
        text.push_str("author A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nm\n");
        assert_eq!(
            parse_commit(text.as_bytes()).unwrap_err().form(),
            form::COMMIT_PARENTS_EXCEEDED
        );

        // Exactly at the bound is inside it.
        let mut text = format!("tree {TREE}\n");
        for parent in &parents[..MAX_COMMIT_PARENTS] {
            text.push_str(&format!("parent {parent}\n"));
        }
        text.push_str("author A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nm\n");
        assert_eq!(
            parse_commit(text.as_bytes()).unwrap().parents.len(),
            MAX_COMMIT_PARENTS
        );
    }

    #[test]
    fn empty_bytes_are_refused_rather_than_producing_an_empty_commit() {
        assert_eq!(
            parse_commit(b"").unwrap_err().form(),
            form::COMMIT_HEADER_MALFORMED
        );
    }
}
