//! The `module_facts` extraction cache (schema v2).
//!
//! This table is **not** part of the evidence graph and is deliberately absent from the
//! canonical dump. It records, per indexed module, the content hash it was last extracted at,
//! the extractor versions that did it, and an opaque payload owned by `nerve-index`.
//!
//! The store treats the payload as text. Its shape is an extractor concern, and pinning it here
//! would put language knowledge in the storage layer.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use crate::error::Result;

/// One cached module row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFactsRow {
    /// Repository-relative path.
    pub rel_path: String,
    /// BLAKE3 of the file's bytes when these facts were produced.
    pub content_hash: String,
    /// Language tag the file was parsed as.
    pub language: String,
    /// Version of `ts-js-structural` that produced them.
    pub structural_version: String,
    /// Version of `ts-js-reference` that produced them.
    pub reference_version: String,
    /// Extractor-owned payload, canonical JSON.
    pub facts: String,
}

/// Read every cached module for a repository, keyed by path.
pub fn load_module_facts(
    conn: &Connection,
    repo_id: &str,
) -> Result<BTreeMap<String, ModuleFactsRow>> {
    let mut stmt = conn.prepare(
        "SELECT rel_path, content_hash, language, structural_version, reference_version, facts
           FROM module_facts WHERE repo_id = ?1 ORDER BY rel_path",
    )?;
    let rows = stmt.query_map(params![repo_id], |row| {
        Ok(ModuleFactsRow {
            rel_path: row.get(0)?,
            content_hash: row.get(1)?,
            language: row.get(2)?,
            structural_version: row.get(3)?,
            reference_version: row.get(4)?,
            facts: row.get(5)?,
        })
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let row = row?;
        out.insert(row.rel_path.clone(), row);
    }
    Ok(out)
}

/// Insert or replace one cached module. Returns the number of rows written.
pub fn upsert_module_facts(
    conn: &Connection,
    repo_id: &str,
    row: &ModuleFactsRow,
) -> Result<usize> {
    Ok(conn.execute(
        "INSERT INTO module_facts
             (repo_id, rel_path, content_hash, language, structural_version,
              reference_version, facts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(repo_id, rel_path) DO UPDATE SET
             content_hash       = excluded.content_hash,
             language           = excluded.language,
             structural_version = excluded.structural_version,
             reference_version  = excluded.reference_version,
             facts              = excluded.facts",
        params![
            repo_id,
            row.rel_path,
            row.content_hash,
            row.language,
            row.structural_version,
            row.reference_version,
            row.facts,
        ],
    )?)
}

/// Delete one cached module. Returns whether a row was removed.
pub fn delete_module_facts(conn: &Connection, repo_id: &str, rel_path: &str) -> Result<bool> {
    let removed = conn.execute(
        "DELETE FROM module_facts WHERE repo_id = ?1 AND rel_path = ?2",
        params![repo_id, rel_path],
    )?;
    Ok(removed > 0)
}
