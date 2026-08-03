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
    /// Python, `.py`.
    ///
    /// Added in Slice 9a, and deliberately **not** folded into the TS/JS family. The two are
    /// handled by different extractors with different ids, because an observation that named
    /// `ts-js-structural` for a Python file would be the Slice 5d-i defect restated for a new
    /// language: an evidence label that can say something false is decoration.
    Python,
}

/// Every extension Nerve indexes, in resolution-preference order.
pub const INDEXED_EXTENSIONS: [&str; 7] = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "py"];

/// Canonical language tag stored on Python entities.
pub const PYTHON_LANGUAGE: &str = "python";

/// The single extension the Python extractor claims.
///
/// `.pyi` is deliberately absent: a stub file declares the same names as the module it shadows,
/// so indexing both would put two `Module` entities on one import target and two symbols behind
/// one qualified name. That is a decision for a later slice, not a default.
pub const PYTHON_EXTENSION: &str = "py";

impl Language {
    /// Map a lower-case file extension to a grammar.
    pub fn from_extension(extension: &str) -> Option<Language> {
        match extension {
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "js" | "mjs" | "cjs" | "jsx" => Some(Language::JavaScript),
            "py" => Some(Language::Python),
            _ => None,
        }
    }

    /// Canonical language tag stored on entities.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Python => PYTHON_LANGUAGE,
        }
    }

    /// True when the grammar has TypeScript-only constructs such as `interface`.
    pub fn has_type_syntax(self) -> bool {
        matches!(self, Language::TypeScript | Language::Tsx)
    }

    /// True when this language is handled by the `ts-js-*` extractors.
    ///
    /// The pipeline dispatches on this rather than on "is it code?", so that adding a language
    /// cannot silently hand its files to an extractor that does not speak it.
    pub fn is_ts_js(self) -> bool {
        matches!(
            self,
            Language::TypeScript | Language::Tsx | Language::JavaScript
        )
    }

    /// True when this language is handled by the `py-structural` extractor.
    pub fn is_python(self) -> bool {
        matches!(self, Language::Python)
    }

    /// The tree-sitter grammar for this language.
    pub fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
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

/// The lower-case extension of a repository-relative path, if it has one.
fn extension_of(rel_path: &str) -> Option<String> {
    let index = rel_path.rfind('.')?;
    Some(rel_path[index + 1..].to_ascii_lowercase())
}

/// True when a repository-relative path names a document.
///
/// Used where only a path is available — change detection, the cache reuse rule, and the T7
/// invariant test — so that "is this a document?" has exactly one answer everywhere.
pub fn path_is_document(rel_path: &str) -> bool {
    extension_of(rel_path)
        .is_some_and(|extension| DOCUMENT_EXTENSIONS.contains(&extension.as_str()))
}

/// True when a repository-relative path names a Python module.
///
/// The path-only counterpart of [`Language::is_python`], for the two places that have a path and
/// no `FileKind`: the cache reuse rule in the pipeline, and specifier re-resolution in
/// [`crate::incremental`]. Stated once here so those two cannot answer it differently.
pub fn path_is_python(rel_path: &str) -> bool {
    extension_of(rel_path).is_some_and(|extension| extension == PYTHON_EXTENSION)
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
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(
            Language::from_extension("pyi"),
            None,
            "a stub file would put a second Module on one import target"
        );
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

    /// Every language belongs to exactly one extractor family.
    ///
    /// The pipeline dispatches on these two predicates, so a language that answered `false` to
    /// both would be discovered, loaded, hashed — and then silently extracted by nobody.
    #[test]
    fn every_language_belongs_to_exactly_one_extractor_family() {
        let pinned: [(Language, bool, bool); 4] = [
            (Language::TypeScript, true, false),
            (Language::Tsx, true, false),
            (Language::JavaScript, true, false),
            (Language::Python, false, true),
        ];
        for (language, ts_js, python) in pinned {
            assert_eq!(language.is_ts_js(), ts_js, "{language:?}");
            assert_eq!(language.is_python(), python, "{language:?}");
            assert!(
                language.is_ts_js() ^ language.is_python(),
                "{language:?} belongs to no extractor family, or to two"
            );
        }
        for extension in INDEXED_EXTENSIONS {
            let language = Language::from_extension(extension).expect("indexed");
            assert!(
                pinned.iter().any(|(pinned, _, _)| *pinned == language),
                "{extension} maps to {language:?}, which is not classified above"
            );
        }
        assert!(!Language::Python.has_type_syntax());
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
        for path in ["src/app.ts", "md", "a.md.ts", "docs/no-extension", "a.py"] {
            assert!(!path_is_document(path), "{path}");
        }
    }

    #[test]
    fn path_python_detection_matches_the_extension_table() {
        for path in ["app.py", "pkg/__init__.py", "a/b/C.PY"] {
            assert!(path_is_python(path), "{path}");
        }
        for path in ["src/app.ts", "py", "a.py.ts", "stub.pyi", "README.md"] {
            assert!(!path_is_python(path), "{path}");
        }
    }

    #[test]
    fn every_grammar_loads() {
        for language in [
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
        ] {
            assert!(language.parser().is_ok(), "{language:?} failed to load");
        }
    }
}
