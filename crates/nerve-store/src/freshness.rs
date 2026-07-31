//! Query-time freshness of an observation.
//!
//! `observation.content_hash` records what the file said when the observation was made
//! (ADR-0003). Whether that is still true is **derived**, never stored: the file is re-hashed
//! on disk and the two hashes are compared.
//!
//! This crate deliberately does not open files. Reading a repository path is a path-safety
//! problem, and the path-safety rules live with the code that owns the repository root
//! (`nerve-index`). So the read is supplied through [`FileProber`] and this module only
//! compares hashes and caches the result per distinct path.

use std::collections::BTreeMap;

/// The outcome of a safety-checked read of one repository-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileProbe {
    /// The file was read; this is the hash of its current bytes.
    Hash(String),
    /// Nothing exists at that path any more.
    Missing,
    /// The path failed the repository path-safety check and was **not** read.
    Refused,
    /// The path passed the safety check but its bytes could not be obtained.
    Unreadable,
}

/// Reads a repository-relative path under the repository's path-safety rules.
///
/// Implementors must treat the path as untrusted input: it comes out of the database, and the
/// database is a file on disk that Nerve does not own exclusively.
pub trait FileProber {
    /// Hash the current contents of `rel_path`, or say why that was not possible.
    fn probe(&self, rel_path: &str) -> FileProbe;
}

/// Whether an observation still describes the file it was taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Freshness {
    /// The file on disk hashes to exactly what the observation recorded.
    Fresh,
    /// The file exists and has changed since the observation was made.
    Stale,
    /// The file the observation points at no longer exists.
    FileMissing,
    /// The path was refused by the path-safety check, so nothing was read.
    Refused,
    /// The path exists and was allowed, but could not be read.
    Unreadable,
}

impl Freshness {
    /// Every value, in declaration order.
    pub const ALL: [Freshness; 5] = [
        Freshness::Fresh,
        Freshness::Stale,
        Freshness::FileMissing,
        Freshness::Refused,
        Freshness::Unreadable,
    ];

    /// Canonical name used in rendered and `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Freshness::Fresh => "fresh",
            Freshness::Stale => "stale",
            Freshness::FileMissing => "file-missing",
            Freshness::Refused => "refused",
            Freshness::Unreadable => "unreadable",
        }
    }
}

impl std::fmt::Display for Freshness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One probe per distinct path, however many observations quote that path.
///
/// A `why` answer routinely carries dozens of observations drawn from a handful of files;
/// hashing the same file once per observation would be the obvious way to make an evidence
/// command feel slow.
pub struct FreshnessCache<'a> {
    prober: &'a dyn FileProber,
    probes: BTreeMap<String, FileProbe>,
}

impl<'a> FreshnessCache<'a> {
    /// Wrap a prober.
    pub fn new(prober: &'a dyn FileProber) -> Self {
        FreshnessCache {
            prober,
            probes: BTreeMap::new(),
        }
    }

    /// Compare `recorded_hash` against the file's current contents.
    pub fn evaluate(&mut self, rel_path: &str, recorded_hash: &str) -> Freshness {
        let probe = match self.probes.get(rel_path) {
            Some(probe) => probe,
            None => {
                let probe = self.prober.probe(rel_path);
                self.probes.entry(rel_path.to_string()).or_insert(probe)
            }
        };
        match probe {
            FileProbe::Hash(current) if current == recorded_hash => Freshness::Fresh,
            FileProbe::Hash(_) => Freshness::Stale,
            FileProbe::Missing => Freshness::FileMissing,
            FileProbe::Refused => Freshness::Refused,
            FileProbe::Unreadable => Freshness::Unreadable,
        }
    }

    /// How many distinct paths were actually probed.
    pub fn files_probed(&self) -> usize {
        self.probes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct CountingProber {
        answers: BTreeMap<String, FileProbe>,
        calls: RefCell<usize>,
    }

    impl FileProber for CountingProber {
        fn probe(&self, rel_path: &str) -> FileProbe {
            *self.calls.borrow_mut() += 1;
            self.answers
                .get(rel_path)
                .cloned()
                .unwrap_or(FileProbe::Missing)
        }
    }

    fn prober() -> CountingProber {
        let mut answers = BTreeMap::new();
        answers.insert("a.ts".to_string(), FileProbe::Hash("h1".to_string()));
        answers.insert("b.ts".to_string(), FileProbe::Refused);
        answers.insert("c.ts".to_string(), FileProbe::Unreadable);
        CountingProber {
            answers,
            calls: RefCell::new(0),
        }
    }

    #[test]
    fn hashes_are_compared_not_stored() {
        let prober = prober();
        let mut cache = FreshnessCache::new(&prober);
        assert_eq!(cache.evaluate("a.ts", "h1"), Freshness::Fresh);
        assert_eq!(cache.evaluate("a.ts", "other"), Freshness::Stale);
    }

    #[test]
    fn every_probe_outcome_maps_to_a_freshness() {
        let prober = prober();
        let mut cache = FreshnessCache::new(&prober);
        assert_eq!(cache.evaluate("b.ts", "h"), Freshness::Refused);
        assert_eq!(cache.evaluate("c.ts", "h"), Freshness::Unreadable);
        assert_eq!(cache.evaluate("gone.ts", "h"), Freshness::FileMissing);
    }

    #[test]
    fn each_distinct_path_is_probed_exactly_once() {
        let prober = prober();
        {
            let mut cache = FreshnessCache::new(&prober);
            for _ in 0..40 {
                cache.evaluate("a.ts", "h1");
                cache.evaluate("b.ts", "h1");
            }
            assert_eq!(cache.files_probed(), 2);
        }
        assert_eq!(*prober.calls.borrow(), 2);
    }

    #[test]
    fn names_are_the_output_contract() {
        let names: Vec<&str> = Freshness::ALL.iter().map(|f| f.as_str()).collect();
        assert_eq!(
            names,
            vec!["fresh", "stale", "file-missing", "refused", "unreadable"]
        );
    }
}
