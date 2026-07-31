//! Language selection.
//!
//! Extension to grammar mapping is the whole of the Slice 1 language extension point. A new
//! language is `(extension set, grammar, extract fn, declared evidence types)`.

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
        assert_eq!(Language::from_extension("md"), None);
        assert_eq!(
            Language::from_extension("TS"),
            None,
            "matching is lowercase"
        );
    }

    #[test]
    fn every_grammar_loads() {
        for language in [Language::TypeScript, Language::Tsx, Language::JavaScript] {
            assert!(language.parser().is_ok(), "{language:?} failed to load");
        }
    }
}
