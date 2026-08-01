//! Language selection, and the code/document split.
//!
//! Extension to grammar mapping is the whole of the Slice 1 language extension point. A new
//! language is `(extension set, grammar, extract fn, declared evidence types)`.
//!
//! Slice 5a adds a second axis. A Markdown file has no grammar and no module semantics, so
//! `Language` alone can no longer say what a discovered file *is*; [`FileKind`] does. The split
//! is not cosmetic — it is what keeps a document out of module resolution and keeps every
//! document-derived observation on the `DOCUMENT_STATED` side of THREAT-MODEL.md T7.

use crate::error::{IndexError, Result};

/// A language Nerve can parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    /// TypeScript, `.ts`.
    TypeScript,
    /// TypeScript with JSX, `.tsx`.
    Tsx,
    /// JavaScript, `.js` / `.mjs` / `.cjs` / `.jsx`.
    JavaScript,
}

/// Every extension Nerve indexes, in resolution-preference order.
pub const INDEXED_EXTENSIONS: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];

impl Language {
    /// Map a lower-case file extension to a grammar.
    pub fn from_extension(extension: &str) -> Option<Language> {
        match extension {
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "js" | "mjs" | "cjs" | "jsx" => Some(Language::JavaScript),
            _ => None,
        }
    }

    /// Canonical language tag stored on entities.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
        }
    }

    /// True when the grammar has TypeScript-only constructs such as `interface`.
    pub fn has_type_syntax(self) -> bool {
        matches!(self, Language::TypeScript | Language::Tsx)
    }

    /// The tree-sitter grammar for this language.
    pub fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        }
    }

    /// A parser configured for this language.
    pub fn parser(self) -> Result<tree_sitter::Parser> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&self.grammar())
            .map_err(|err| IndexError::Parser(format!("{}: {err}", self.as_str())))?;
        Ok(parser)
    }
}

/// Canonical language tag stored on document entities.
pub const MARKDOWN_LANGUAGE: &str = "markdown";

/// Every document extension Nerve indexes.
pub const DOCUMENT_EXTENSIONS: [&str; 2] = ["md", "markdown"];

/// What a discovered file is, as far as extraction is concerned.
///
/// The two arms are handled by different extractors and never mix: code goes to
/// `ts-js-structural` / `ts-js-reference`, documents to `md-structural`. Nothing in module
/// resolution ever sees a [`FileKind::Doc`] path, which is why a specifier can never resolve to
/// a document and produce an `IMPORTS` edge to an entity no extractor created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    /// Source Nerve parses with a grammar.
    Code(Language),
    /// Prose Nerve scans for structure.
    Doc,
}

impl FileKind {
    /// Map a lower-case file extension to a file kind, or `None` when Nerve does not index it.
    pub fn from_extension(extension: &str) -> Option<FileKind> {
        if let Some(language) = Language::from_extension(extension) {
            return Some(FileKind::Code(language));
        }
        if DOCUMENT_EXTENSIONS.contains(&extension) {
            return Some(FileKind::Doc);
        }
        None
    }

    /// The grammar this file is parsed with, or `None` for a document.
    pub fn language(self) -> Option<Language> {
        match self {
            FileKind::Code(language) => Some(language),
            FileKind::Doc => None,
        }
    }

    /// Canonical language tag stored on entities derived from this file.
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Code(language) => language.as_str(),
            FileKind::Doc => MARKDOWN_LANGUAGE,
        }
    }

    /// True when the file is prose rather than source.
    pub fn is_doc(self) -> bool {
        matches!(self, FileKind::Doc)
    }
}

/// True when a repository-relative path names a document.
///
/// Used where only a path is available — change detection, the cache reuse rule, and the T7
/// invariant test — so that "is this a document?" has exactly one answer everywhere.
pub fn path_is_document(rel_path: &str) -> bool {
    let Some(index) = rel_path.rfind('.') else {
        return false;
    };
    let extension = rel_path[index + 1..].to_ascii_lowercase();
    DOCUMENT_EXTENSIONS.contains(&extension.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_mapping_is_exhaustive_over_indexed_extensions() {
        for extension in INDEXED_EXTENSIONS {
            assert!(
                Language::from_extension(extension).is_some(),
                "{extension} has no grammar"
            );
        }
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::Tsx));
        assert_eq!(Language::from_extension("cjs"), Some(Language::JavaScript));
        assert_eq!(
            Language::from_extension("md"),
            None,
            "Markdown has no grammar; it is a FileKind::Doc"
        );
        assert_eq!(
            Language::from_extension("TS"),
            None,
            "matching is lowercase"
        );
    }

    #[test]
    fn file_kinds_cover_code_and_documents_and_nothing_else() {
        for extension in INDEXED_EXTENSIONS {
            assert!(matches!(
                FileKind::from_extension(extension),
                Some(FileKind::Code(_))
            ));
        }
        for extension in DOCUMENT_EXTENSIONS {
            assert_eq!(FileKind::from_extension(extension), Some(FileKind::Doc));
        }
        assert_eq!(FileKind::from_extension("txt"), None);
        assert_eq!(FileKind::from_extension("rst"), None);
        assert_eq!(
            FileKind::from_extension("MD"),
            None,
            "matching is lowercase"
        );
        assert_eq!(FileKind::Doc.as_str(), MARKDOWN_LANGUAGE);
        assert_eq!(FileKind::Code(Language::Tsx).as_str(), "tsx");
        assert!(FileKind::Doc.is_doc());
        assert!(!FileKind::Code(Language::TypeScript).is_doc());
        assert!(FileKind::Doc.language().is_none());
    }

    #[test]
    fn path_document_detection_matches_the_extension_table() {
        for path in ["README.md", "docs/a.markdown", "a/b/C.MD", "x.Markdown"] {
            assert!(path_is_document(path), "{path}");
        }
        for path in ["src/app.ts", "md", "a.md.ts", "docs/no-extension"] {
            assert!(!path_is_document(path), "{path}");
        }
    }

    #[test]
    fn every_grammar_loads() {
        for language in [Language::TypeScript, Language::Tsx, Language::JavaScript] {
            assert!(language.parser().is_ok(), "{language:?} failed to load");
        }
    }
}
