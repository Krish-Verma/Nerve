//! The one place `flate2` is called, with the output bound applied **as the stream is read**.
//!
//! # Why this is a module of its own
//!
//! Every byte of every Git object arrives zlib-deflated —
//! `docs/plans/slice-12-git-object-access-analysis.md` §2 measured `78 01` on this repository's own
//! loose objects — so there is exactly one gate every object must pass through, and putting it in
//! one file is what makes the bound reviewable.
//!
//! # The bound is applied while inflating, not afterwards
//!
//! This is the security substance of Slice 12a. A deflate stream describes its own output size, and
//! the ratio is unbounded: a few hundred kilobytes of input can name gigabytes of output. The
//! obvious-looking implementation —
//!
//! ```text
//! let data = decoder.read_to_end(&mut out)?;   // wrong
//! if out.len() > MAX_OBJECT_BYTES { refuse }   // too late, the bomb is already resident
//! ```
//!
//! — has already allocated the attack by the time it checks. So [`inflate_bounded`] drives the
//! decompressor itself and owns the output buffer:
//!
//! 1. **The output buffer's capacity is capped at `limit + 1`.** It grows by doubling but never
//!    reserves past that cap, so no allocation in this function can exceed it. One byte past the
//!    limit is enough to *know* the limit was exceeded, and it is the smallest amount of evidence
//!    that distinguishes "exactly at the bound" from "over it".
//! 2. **The refusal fires the moment the buffer is full**, not after the stream finishes. The
//!    decompressor is asked for output in chunks and is never given somewhere to put more than the
//!    bound allows.
//!
//! `crates/nerve-index/tests/gitobj_bomb.rs` measures this with a tracking global allocator: it
//! builds a bomb declaring **eight times** [`MAX_OBJECT_BYTES`] and requires peak heap growth to
//! stay under **twice** it. An inflate-then-check implementation cannot pass that test, which is
//! the point of writing it that way round.
//!
//! # Why the low-level `Decompress` API and not `read::ZlibDecoder` + `Read::take`
//!
//! `Read::take` bounds the output and was the first implementation here. It was replaced because of
//! a **measured** property of `flate2 1.1.9`: `read::ZlibDecoder` on a stream that ends before the
//! deflate data does returns `Ok(0)` — end of file — rather than an error. So
//! `deflate(&vec![b'x'; 4096])` cut in half inflated to `Ok([])`, a *silent empty prefix* of a real
//! object. For a reader whose whole job is not handing a caller wrong bytes, a truncated stream
//! reported as a complete short one is a worse failure than an unbounded one: nothing downstream can
//! tell it from an empty file.
//!
//! Driving [`flate2::Decompress`] directly fixes that and tightens the bound at the same time. The
//! stream must reach [`flate2::Status::StreamEnd`], and the output buffer's capacity is this
//! function's own to cap, rather than `read_to_end`'s to double past the ceiling.

use std::io::Read;

use flate2::{Decompress, FlushDecompress, Status};

use super::{Error, Result};

/// The largest inflated object this reader will produce, in bytes.
///
/// 64 MiB. A source file Nerve indexes is bounded at 2 MiB
/// ([`crate::config::DEFAULT_MAX_FILE_BYTES`]), so this is generous by a factor of 32 for the
/// blobs the historical model will actually want, while staying small enough that inflating one
/// object never becomes the largest allocation in the process.
///
/// It applies to *every* object: a loose object, a pack entry, a delta's declared result, and each
/// intermediate reconstruction in a delta chain.
pub const MAX_OBJECT_BYTES: usize = 64 * 1024 * 1024;

/// Bytes read from the decoder per iteration.
///
/// Fixed and stack-resident, so the read granularity is not itself a function of attacker input.
const CHUNK_BYTES: usize = 16 * 1024;

/// The smallest capacity the output buffer ever asks for.
const MIN_CAPACITY: usize = 8 * 1024;

/// Inflate a zlib stream, refusing at `limit` bytes of output rather than allocating past it.
///
/// Returns [`Error::ObjectTooLarge`] the moment the output would exceed `limit`, and
/// [`Error::Inflate`] for a stream that is not valid zlib **or that ends before the zlib stream
/// does**. Never panics, and never allocates more than `limit + 1` bytes of output.
pub fn inflate_bounded<R: Read>(mut input: R, limit: usize) -> Result<Vec<u8>> {
    // The ceiling the output buffer is held to. `limit + 1` rather than `limit`, because a stream
    // whose output is exactly `limit` bytes is inside the bound and must still be readable.
    let ceiling = limit.saturating_add(1);
    let mut decompressor = Decompress::new(true);
    let mut out: Vec<u8> = Vec::new();
    let mut buffer = [0u8; CHUNK_BYTES];
    let mut filled = 0usize;
    let mut position = 0usize;
    let mut input_done = false;

    loop {
        if out.len() == out.capacity() {
            if out.len() >= ceiling {
                // The buffer is full at `limit + 1`, so the object is over the bound. Refused
                // holding one byte more than the bound and nothing else.
                return Err(Error::ObjectTooLarge {
                    limit,
                    at_least: out.len(),
                });
            }
            // Never asks for more than the ceiling, so the reservation itself is bounded.
            let extra = (ceiling - out.len()).min(CHUNK_BYTES);
            reserve_capped(&mut out, extra, ceiling);
        }
        if position == filled && !input_done {
            filled = input
                .read(&mut buffer)
                .map_err(|err| Error::Inflate(err.to_string()))?;
            position = 0;
            input_done = filled == 0;
        }

        let before_in = decompressor.total_in();
        let before_out = decompressor.total_out();
        let flush = if input_done {
            FlushDecompress::Finish
        } else {
            FlushDecompress::None
        };
        let status = decompressor
            .decompress_vec(&buffer[position..filled], &mut out, flush)
            .map_err(|err| Error::Inflate(err.to_string()))?;
        position += (decompressor.total_in() - before_in) as usize;

        if status == Status::StreamEnd {
            break;
        }
        if decompressor.total_in() == before_in && decompressor.total_out() == before_out {
            // No input consumed and no output produced, with room for both. The stream cannot
            // finish, which for a Git object means the file was cut short. Reported rather than
            // returned as a short success: a truncated stream read as a complete one is the failure
            // this whole module exists to avoid.
            return Err(Error::Inflate(
                "the stream ended before the zlib stream did".to_string(),
            ));
        }
    }

    if out.len() > limit {
        return Err(Error::ObjectTooLarge {
            limit,
            at_least: out.len(),
        });
    }
    Ok(out)
}

/// Grow `out` to hold `extra` more bytes, doubling but never reserving past `cap`.
///
/// Two rules, and the second one is not cosmetic:
///
/// 1. **Never reserve past `cap`.** `Vec`'s own growth would double past it, so a 64 MiB ceiling
///    would see a 128 MiB request while the 64 MiB buffer was still live.
/// 2. **Once doubling would land within one chunk of `cap`, jump straight to `cap`.** Otherwise the
///    last two growths are `cap`-rounded-down and then `cap`, which is two large reallocations
///    instead of one, and the peak is the sum of the two largest live blocks rather than of the two
///    largest halves. Measured: without this, the bomb test's peak was 128 MiB + 43 KB against a
///    64 MiB bound; with it, 96 MiB.
fn reserve_capped(out: &mut Vec<u8>, extra: usize, cap: usize) {
    let needed = out.len() + extra;
    if needed <= out.capacity() {
        return;
    }
    let doubled = out.capacity().max(MIN_CAPACITY).saturating_mul(2);
    let target = if doubled >= cap.saturating_sub(CHUNK_BYTES) {
        cap
    } else {
        doubled
    };
    let target = target.max(needed).min(cap.max(needed));
    out.reserve_exact(target - out.len());
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

    #[test]
    fn a_stream_inside_the_bound_round_trips() {
        let payload = b"tree 4\0abcd".to_vec();
        let compressed = deflate(&payload);
        assert_eq!(
            inflate_bounded(compressed.as_slice(), MAX_OBJECT_BYTES).unwrap(),
            payload
        );
    }

    /// Exactly at the bound is inside it; one byte past is not. Both halves, because an off-by-one
    /// here either refuses legitimate objects or admits one byte of every attack.
    #[test]
    fn the_bound_is_inclusive_and_one_byte_past_it_is_refused() {
        let at_bound = vec![b'x'; 1024];
        let compressed = deflate(&at_bound);
        assert_eq!(
            inflate_bounded(compressed.as_slice(), 1024).unwrap().len(),
            1024
        );

        let past_bound = vec![b'x'; 1025];
        let compressed = deflate(&past_bound);
        let error = inflate_bounded(compressed.as_slice(), 1024).unwrap_err();
        assert!(
            matches!(error, Error::ObjectTooLarge { limit: 1024, .. }),
            "expected ObjectTooLarge, got {error:?}"
        );
    }

    #[test]
    fn a_stream_that_is_not_zlib_is_refused_rather_than_guessed_at() {
        let error = inflate_bounded(&b"PACK\0\0\0\x02"[..], MAX_OBJECT_BYTES).unwrap_err();
        assert!(matches!(error, Error::Inflate(_)), "got {error:?}");
        assert_eq!(error.form(), super::super::form::INFLATE_FAILED);
    }

    /// **The measurement that chose the implementation.** `flate2::read::ZlibDecoder` returns
    /// `Ok(0)` here, so a `read_to_end`-shaped reader hands back a silent empty prefix of a real
    /// object. Every truncation of a real stream must be a refusal instead.
    #[test]
    fn a_truncated_stream_is_refused_rather_than_returning_a_prefix() {
        let payload = vec![b'x'; 4096];
        let compressed = deflate(&payload);
        for cut in [1usize, 2, 3, compressed.len() / 2, compressed.len() - 1] {
            let error = inflate_bounded(&compressed[..cut], MAX_OBJECT_BYTES).unwrap_err();
            assert!(
                matches!(error, Error::Inflate(_)),
                "cutting to {cut} bytes gave {error:?} instead of a refusal"
            );
        }
        // And the whole stream still works, so the check above is not simply refusing everything.
        assert_eq!(
            inflate_bounded(compressed.as_slice(), MAX_OBJECT_BYTES).unwrap(),
            payload
        );
    }

    /// Trailing bytes after a complete stream are not this function's business: a pack entry is
    /// followed by the next entry, so the stream ends where zlib says it ends.
    #[test]
    fn bytes_after_the_end_of_the_stream_are_ignored() {
        let mut compressed = deflate(b"hello");
        compressed.extend_from_slice(b"the next pack entry starts here");
        assert_eq!(
            inflate_bounded(compressed.as_slice(), MAX_OBJECT_BYTES).unwrap(),
            b"hello"
        );
    }

    /// The refusal must happen without the whole output ever existing. Here it is checked by
    /// capacity — the buffer the refusal is holding — rather than by a global allocator; the
    /// allocator measurement is in `tests/gitobj_bomb.rs`, which is a separate binary so nothing
    /// runs in parallel with it.
    #[test]
    fn a_bomb_is_refused_without_the_buffer_ever_exceeding_the_bound() {
        let limit = 256 * 1024;
        // Eight times the limit, all zeros, which deflate stores in a few hundred bytes.
        let bomb = deflate(&vec![0u8; limit * 8]);
        assert!(
            bomb.len() < limit,
            "the bomb must be small relative to its output, or it proves nothing: {} bytes",
            bomb.len()
        );
        let error = inflate_bounded(bomb.as_slice(), limit).unwrap_err();
        match error {
            Error::ObjectTooLarge { at_least, .. } => assert_eq!(
                at_least,
                limit + 1,
                "the refusal must fire at limit + 1, not after the whole stream"
            ),
            other => panic!("expected ObjectTooLarge, got {other:?}"),
        }
    }

    /// The growth policy, driven exactly as [`inflate_bounded`] drives it: the reservation is always
    /// clamped to what is left under the cap, so capacity never passes it.
    #[test]
    fn reserve_capped_never_reserves_past_the_cap() {
        let cap = 40_000usize;
        let mut out: Vec<u8> = Vec::new();
        while out.len() < cap {
            if out.len() == out.capacity() {
                let extra = (cap - out.len()).min(CHUNK_BYTES);
                reserve_capped(&mut out, extra, cap);
                assert!(
                    out.capacity() <= cap,
                    "capacity {} exceeded the cap {cap}",
                    out.capacity()
                );
                assert!(out.capacity() > out.len(), "no room was made");
            }
            // Stand in for the decompressor filling the spare capacity.
            let room = out.capacity() - out.len();
            out.resize(out.len() + room, 0);
        }
        assert_eq!(out.len(), cap);
        assert_eq!(out.capacity(), cap);
    }
}
