//! `.nerve/` layout and `config.toml`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IndexError, Result};

/// Directory holding the index, inside the repository.
pub const NERVE_DIR: &str = ".nerve";
/// Config file name.
pub const CONFIG_FILE: &str = "config.toml";
/// Database file name.
pub const DB_FILE: &str = "nerve.db";
/// Default per-file byte ceiling: 2 MiB.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Built-in secret deny-list (SECURITY.md).
///
/// Applied to the file name of every discovered path **before** its contents are read, so a
/// denied file is never loaded into memory. Patterns support `*` as a wildcard.
pub const SECRET_DENY_PATTERNS: [&str; 15] = [
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.keystore",
    "*.jks",
    "id_rsa*",
    "id_ed25519*",
    ".npmrc",
    ".netrc",
    ".pgpass",
    "credentials",
    "secrets.*",
];

/// Directories never descended into, regardless of ignore files.
pub const PRUNED_DIRECTORIES: [&str; 3] = [".git", NERVE_DIR, "node_modules"];

/// Indexing knobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSettings {
    /// Files larger than this are skipped and counted as failures.
    pub max_file_bytes: u64,
}

impl Default for IndexSettings {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }
}

/// Security knobs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecuritySettings {
    /// Additional deny-list patterns, appended to [`SECRET_DENY_PATTERNS`].
    #[serde(default)]
    pub extra_deny_patterns: Vec<String>,
}

/// Contents of `.nerve/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Schema version this config was written for.
    pub schema_version: i64,
    /// Stable project identifier. Not derived from any path, so the index survives moves.
    pub project_id: String,
    /// When `nerve init` created this config.
    pub created_at: String,
    /// Indexing knobs.
    #[serde(default)]
    pub index: IndexSettings,
    /// Security knobs.
    #[serde(default)]
    pub security: SecuritySettings,
}

impl Config {
    /// Every deny-list pattern in effect: built-in plus user-supplied.
    pub fn deny_patterns(&self) -> Vec<String> {
        SECRET_DENY_PATTERNS
            .iter()
            .map(|p| (*p).to_string())
            .chain(self.security.extra_deny_patterns.iter().cloned())
            .collect()
    }

    /// Read `<root>/.nerve/config.toml`.
    pub fn load(root: &Path) -> Result<Config> {
        let path = config_path(root);
        let text = std::fs::read_to_string(&path).map_err(|err| IndexError::Config {
            path: path.clone(),
            message: err.to_string(),
        })?;
        toml::from_str(&text).map_err(|err| IndexError::Config {
            path,
            message: err.to_string(),
        })
    }

    /// Write `<root>/.nerve/config.toml`.
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = config_path(root);
        let text = toml::to_string_pretty(self).map_err(|err| IndexError::Config {
            path: path.clone(),
            message: err.to_string(),
        })?;
        std::fs::write(&path, text)?;
        Ok(())
    }
}

/// `<root>/.nerve`
pub fn nerve_dir(root: &Path) -> PathBuf {
    root.join(NERVE_DIR)
}

/// `<root>/.nerve/config.toml`
pub fn config_path(root: &Path) -> PathBuf {
    nerve_dir(root).join(CONFIG_FILE)
}

/// `<root>/.nerve/nerve.db`
pub fn db_path(root: &Path) -> PathBuf {
    nerve_dir(root).join(DB_FILE)
}

/// Match a file name against a deny-list pattern.
///
/// Supports `*` (any run of characters, including empty) anywhere in the pattern. Matching is
/// case-sensitive and applies to the file name only, never to a whole path, so a pattern can
/// never be escaped by nesting.
pub fn pattern_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut resume = 0usize;

    while n < name.len() {
        if p < pattern.len() && pattern[p] == name[n] {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = n;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            resume += 1;
            n = resume;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// True when a file name is denied by any of the supplied patterns.
pub fn is_denied(name: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| pattern_matches(pattern, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_patterns_deny_the_documented_names() {
        let patterns: Vec<String> = SECRET_DENY_PATTERNS.iter().map(|p| p.to_string()).collect();
        for name in [
            ".env",
            ".env.local",
            ".env.production",
            "server.pem",
            "private.key",
            "cert.p12",
            "cert.pfx",
            "release.keystore",
            "debug.jks",
            "id_rsa",
            "id_rsa.pub",
            "id_ed25519",
            "id_ed25519.pub",
            ".npmrc",
            ".netrc",
            ".pgpass",
            "credentials",
            "secrets.ts",
            "secrets.json",
        ] {
            assert!(is_denied(name, &patterns), "{name} should be denied");
        }
    }

    #[test]
    fn builtin_patterns_allow_ordinary_source_files() {
        let patterns: Vec<String> = SECRET_DENY_PATTERNS.iter().map(|p| p.to_string()).collect();
        for name in [
            "math.ts",
            "index.tsx",
            "legacy.cjs",
            "environment.ts",
            "keyboard.ts",
            "credentials.service.ts",
            "monkey.ts",
        ] {
            assert!(!is_denied(name, &patterns), "{name} should be allowed");
        }
    }

    #[test]
    fn wildcard_matching_handles_edges() {
        assert!(pattern_matches("*", ""));
        assert!(pattern_matches("*.pem", "a.pem"));
        assert!(!pattern_matches("*.pem", "pem"));
        assert!(pattern_matches("a*b*c", "abc"));
        assert!(pattern_matches("a*b*c", "axxbyyc"));
        assert!(!pattern_matches("a*b*c", "acb"));
        assert!(pattern_matches(".env.*", ".env."));
        assert!(!pattern_matches(".env.*", ".env"));
    }

    #[test]
    fn user_patterns_extend_the_builtin_list() {
        let config = Config {
            schema_version: 1,
            project_id: "p".into(),
            created_at: "t".into(),
            index: IndexSettings::default(),
            security: SecuritySettings {
                extra_deny_patterns: vec!["*.secret.ts".into()],
            },
        };
        let patterns = config.deny_patterns();
        assert!(is_denied("tokens.secret.ts", &patterns));
        assert!(is_denied(".env", &patterns));
        assert!(!is_denied("tokens.ts", &patterns));
    }

    #[test]
    fn config_round_trips_through_toml() {
        let config = Config {
            schema_version: 1,
            project_id: "00000000000000000000000000000001".into(),
            created_at: "2026-07-31T00:00:00Z".into(),
            index: IndexSettings::default(),
            security: SecuritySettings::default(),
        };
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed, config);
    }
}
