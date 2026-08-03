//! The decompression bomb, **generated from the bound it attacks**, with peak allocation measured.
//!
//! # Why this is its own test binary
//!
//! It installs a tracking `#[global_allocator]` and asserts a *peak*. Anything running in parallel
//! would contribute allocations to that peak, so this file contains exactly one test and no other
//! file shares its process.
//!
//! # Why the payload is generated and not committed
//!
//! `CLAUDE.md` §9 forbids committing generated artifacts, and Slice 11a-i measured the sharper
//! reason: `fixtures/trace-hostile` shipped four artifacts carrying placeholder tokens that nothing
//! expanded, so `__PAD_STRING__` reached the parser as fourteen ASCII bytes and four attacks tested
//! nothing while a green suite reported them as passing. A payload derived from
//! [`nerve_index::gitobj::MAX_OBJECT_BYTES`] is always one byte past whatever the bound currently
//! says, so tightening the bound cannot disarm its own attack. See
//! `crates/nerve-index/tests/trace.rs::stage_hostile` for the pattern this follows.
//!
//! # What the measurement proves
//!
//! The bomb's inflated output is **eight times** the bound. The assertion is that peak heap growth
//! stays under **twice** it. An `inflate-then-check-the-length` implementation has to hold all eight
//! times the bound to reach its check, so it cannot pass — which is the whole point of measuring a
//! peak rather than asserting that an error came back.
//!
//! The accounting is deliberately pessimistic about reallocation: a growing `realloc` is charged as
//! though the new block were live alongside the old one, because the system allocator may in fact
//! hold both while it copies. That can only make the assertion harder to satisfy.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use nerve_index::gitobj::{form, inflate_bounded, read_loose, MAX_OBJECT_BYTES};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Tracking;

fn charge(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

fn release(bytes: usize) {
    LIVE.fetch_sub(bytes, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() {
            charge(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc_zeroed(layout);
        if !pointer.is_null() {
            charge(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        release(layout.size());
        System.dealloc(pointer, layout);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Charged before the old block is released, so a growing reallocation is recorded as though
        // both blocks were live at once. Pessimistic on purpose: the assertion below must not pass
        // because the accounting was generous.
        charge(new_size);
        let new_pointer = System.realloc(pointer, layout, new_size);
        if new_pointer.is_null() {
            release(new_size);
            return new_pointer;
        }
        release(layout.size());
        new_pointer
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

/// Deflate `length` zero bytes without ever holding `length` bytes.
///
/// Fed a megabyte at a time, so building an eight-times-the-bound bomb does not itself allocate
/// eight times the bound and quietly satisfy the measurement it exists to make.
fn zero_bomb(length: usize) -> Vec<u8> {
    const CHUNK: usize = 1024 * 1024;
    let zeros = vec![0u8; CHUNK];
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let mut written = 0usize;
    while written < length {
        let want = CHUNK.min(length - written);
        encoder.write_all(&zeros[..want]).expect("writing to a Vec");
        written += want;
    }
    encoder.finish().expect("finishing a Vec encoder")
}

#[test]
fn a_decompression_bomb_is_refused_during_inflate_with_bounded_peak_allocation() {
    let declared = MAX_OBJECT_BYTES * 8;
    let bomb = zero_bomb(declared);
    assert!(
        bomb.len() < 4 * 1024 * 1024,
        "the bomb must be small relative to its output or it demonstrates no amplification: \
         {} bytes in, {declared} out",
        bomb.len()
    );

    // ---- phase one: the raw stream ----
    let allowance = MAX_OBJECT_BYTES * 2;
    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    let error = inflate_bounded(bomb.as_slice(), MAX_OBJECT_BYTES)
        .expect_err("a bomb eight times the bound must be refused");
    let stream_peak = PEAK.load(Ordering::Relaxed);

    assert_eq!(error.form(), form::OBJECT_TOO_LARGE, "{error}");
    assert!(
        error.to_string().contains(&MAX_OBJECT_BYTES.to_string()),
        "the refusal must name the bound: {error}"
    );
    assert!(
        stream_peak <= allowance,
        "peak allocation was {stream_peak} bytes for a bound of {MAX_OBJECT_BYTES}; the refusal \
         must fire during inflate, not after the output exists"
    );
    assert!(
        stream_peak >= MAX_OBJECT_BYTES,
        "peak allocation was only {stream_peak} bytes, which is less than the bound — the bomb \
         cannot have been inflated at all, so this measurement proves nothing"
    );

    // ---- phase two: the same bomb as a loose object file, through the file reader ----
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("bomb");
    // A header declaring a small object, followed by far more than the bound. The header is never
    // reached: the inflate refuses first, which is what "bounded as it streams" means.
    let mut raw = b"blob 4\0".to_vec();
    raw.extend_from_slice(&bomb);
    let mut file = std::fs::File::create(&path).expect("a temporary file");
    // The whole object must be one zlib stream, so the bomb is rebuilt with its header inside it.
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"blob 4\0").expect("writing to a Vec");
    let zeros = vec![0u8; 1024 * 1024];
    let mut written = 0usize;
    while written < declared {
        let want = zeros.len().min(declared - written);
        encoder.write_all(&zeros[..want]).expect("writing to a Vec");
        written += want;
    }
    let loose_bomb = encoder.finish().expect("finishing a Vec encoder");
    file.write_all(&loose_bomb).expect("writing the fixture");
    drop(file);

    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    let error = read_loose(&path).expect_err("a loose bomb must be refused");
    let loose_peak = PEAK.load(Ordering::Relaxed);

    assert_eq!(error.form(), form::OBJECT_TOO_LARGE, "{error}");
    assert!(
        loose_peak <= allowance,
        "peak allocation while reading a loose bomb was {loose_peak} bytes for a bound of \
         {MAX_OBJECT_BYTES}"
    );
    assert!(
        loose_peak >= MAX_OBJECT_BYTES,
        "peak allocation was only {loose_peak} bytes, so nothing was inflated"
    );
}
