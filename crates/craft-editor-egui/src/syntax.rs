//! Native syntax tokens over exact source bytes, independent of canonical document data.
//!
//! Grammars are immutable versioned Sublime definitions interpreted by syntect. Unknown imported
//! language identifiers and still-missing required grammars are different observable states; neither
//! is rewritten to a different stored language. This module never emits HTML or changes source text.
use std::sync::OnceLock;

use craft_editor_model::ByteOffset;
use syntect::{
    easy::ScopeRangeIterator,
    parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

/// Stable content-free highlighting failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The pinned grammar engine could not parse one source line.
    #[error("syntax_parse_failed")]
    Parse,
    /// A grammar emitted an invalid scope-stack operation.
    #[error("syntax_scope_failed")]
    Scope,
    /// A token did not lie on exact UTF-8 boundaries or cover the source partition.
    #[error("syntax_token_range_invalid")]
    InvalidTokenRange,
}
/// Native highlighting result without manuscript text in errors.
pub type Result<T> = std::result::Result<T, Error>;

/// Semantic color role; tokens never change font metrics, text or canonical marks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    /// Ordinary unclassified source.
    Plain,
    /// Comment or documentation text.
    Comment,
    /// Quoted/raw string or character source.
    String,
    /// Numeric, boolean or named literal constant.
    Constant,
    /// Language keyword, storage type or modifier.
    Keyword,
    /// Function/method name or built-in function.
    Function,
    /// Type, class or built-in type name.
    Type,
    /// Operator or delimiter punctuation.
    Punctuation,
    /// Attribute, property or dictionary key.
    Attribute,
    /// Source that the grammar explicitly classifies as invalid.
    Invalid,
}

/// One nonempty exact half-open UTF-8 byte range in the unchanged original source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenSpan {
    /// Inclusive source byte offset; never a scalar or UTF-16 index.
    pub from: ByteOffset,
    /// Exclusive source byte offset.
    pub to: ByteOffset,
    /// Semantic foreground role.
    pub kind: TokenKind,
}
impl TokenSpan {
    /// Borrow exact source bytes after checking both UTF-8 boundaries.
    pub fn text<'a>(&self, source: &'a str) -> Result<&'a str> {
        if self.from.0 >= self.to.0 {
            return Err(Error::InvalidTokenRange);
        }
        source
            .get(self.from.0..self.to.0)
            .ok_or(Error::InvalidTokenRange)
    }
}

/// Distinguishes a grammar-backed result from intentional plain text and required gaps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Coverage {
    /// The author explicitly selected plain text.
    PlainText,
    /// A versioned native grammar was actually selected.
    Highlighted {
        /// Exact grammar name from the embedded native bundle.
        grammar: String,
    },
    /// An original advertised format still needs its native grammar; release-blocking.
    MissingRequiredGrammar {
        /// Canonical lookup identifier, never a rewritten stored attribute.
        language: String,
    },
    /// An imported arbitrary language identifier has no known grammar; preserve it and its text.
    UnknownLanguage {
        /// Normalized lookup identifier only; the caller retains the exact original attribute.
        language: String,
    },
}

/// Highlight output partitions the source exactly, including whitespace and line endings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Highlighted {
    /// Actual grammar coverage; callers must not treat a required gap as completed parity.
    pub coverage: Coverage,
    /// Ordered nonempty ranges covering every source byte exactly once.
    pub tokens: Vec<TokenSpan>,
}
impl Highlighted {
    /// Check exact partition and UTF-8 boundaries before applying tokens to any layout engine.
    pub fn validate(&self, source: &str) -> Result<()> {
        let mut next = 0;
        for token in &self.tokens {
            if token.from.0 != next {
                return Err(Error::InvalidTokenRange);
            }
            token.text(source)?;
            next = token.to.0;
        }
        if next != source.len() {
            return Err(Error::InvalidTokenRange);
        }
        Ok(())
    }
}

/// Host-surface-aware token foreground colors; no document background or font is overridden.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// Unclassified source foreground.
    pub plain: [u8; 3],
    /// Comment/documentation foreground.
    pub comment: [u8; 3],
    /// String/character foreground.
    pub string: [u8; 3],
    /// Literal foreground.
    pub constant: [u8; 3],
    /// Keyword foreground.
    pub keyword: [u8; 3],
    /// Function/method foreground.
    pub function: [u8; 3],
    /// Type/class foreground.
    pub type_name: [u8; 3],
    /// Punctuation/operator foreground.
    pub punctuation: [u8; 3],
    /// Attribute/key foreground.
    pub attribute: [u8; 3],
    /// Invalid-source foreground.
    pub invalid: [u8; 3],
}
impl Palette {
    /// Derive readable Light/Dark token colors from the actual host surface, including palettes.
    /// Each role is moved toward the host foreground until it reaches a 4.5:1 contrast ratio,
    /// where the host foreground itself permits it. The document's background remains unchanged.
    pub fn for_surface(foreground: [u8; 3], background: [u8; 3]) -> Self {
        let dark = luminance(background) < 0.35;
        let colors = if dark {
            [
                [145, 155, 174],
                [137, 197, 163],
                [225, 171, 119],
                [197, 160, 226],
                [133, 179, 238],
                [230, 195, 134],
                [175, 185, 201],
                [230, 181, 137],
                [241, 139, 139],
            ]
        } else {
            [
                [87, 103, 122],
                [37, 112, 91],
                [151, 77, 42],
                [128, 67, 153],
                [42, 91, 154],
                [130, 88, 26],
                [82, 98, 118],
                [131, 75, 38],
                [179, 48, 58],
            ]
        };
        let adjusted = colors.map(|color| readable(color, foreground, background));
        Self {
            plain: foreground,
            comment: adjusted[0],
            string: adjusted[1],
            constant: adjusted[2],
            keyword: adjusted[3],
            function: adjusted[4],
            type_name: adjusted[5],
            punctuation: adjusted[6],
            attribute: adjusted[7],
            invalid: adjusted[8],
        }
    }
    /// Resolve one semantic role without altering any font property.
    pub fn color(&self, kind: TokenKind) -> [u8; 3] {
        match kind {
            TokenKind::Plain => self.plain,
            TokenKind::Comment => self.comment,
            TokenKind::String => self.string,
            TokenKind::Constant => self.constant,
            TokenKind::Keyword => self.keyword,
            TokenKind::Function => self.function,
            TokenKind::Type => self.type_name,
            TokenKind::Punctuation => self.punctuation,
            TokenKind::Attribute => self.attribute,
            TokenKind::Invalid => self.invalid,
        }
    }
}
fn luminance(color: [u8; 3]) -> f32 {
    fn linear(value: u8) -> f32 {
        let s = f32::from(value) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}
fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn readable(color: [u8; 3], foreground: [u8; 3], background: [u8; 3]) -> [u8; 3] {
    if contrast(color, background) >= 4.5 {
        return color;
    }
    for step in 1..=16 {
        let candidate = std::array::from_fn(|i| {
            ((u16::from(color[i]) * (16 - step) + u16::from(foreground[i]) * step) / 16) as u8
        });
        if contrast(candidate, background) >= 4.5 {
            return candidate;
        }
    }
    foreground
}

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}
/// Full original source option set, including intentional plain text; order matches the source UI.
pub const REQUIRED_LANGUAGES: &[&str] = &[
    "",
    "go",
    "javascript",
    "typescript",
    "html",
    "xml",
    "css",
    "scss",
    "sass",
    "less",
    "json",
    "yaml",
    "markdown",
    "bash",
    "powershell",
    "python",
    "java",
    "kotlin",
    "c",
    "cpp",
    "csharp",
    "rust",
    "swift",
    "objectivec",
    "php",
    "ruby",
    "lua",
    "sql",
    "r",
    "perl",
    "vbnet",
    "basic",
    "arduino",
    "makefile",
    "ini",
    "diff",
    "jsx",
    "tsx",
    "graphql",
    "dockerfile",
    "hcl",
    "terraform",
    "toml",
    "protobuf",
    "dart",
    "scala",
    "clojure",
    "elixir",
    "erlang",
    "haskell",
    "fsharp",
    "ocaml",
    "groovy",
    "gradle",
    "julia",
    "matlab",
    "fortran",
    "pascal",
    "cobol",
    "lisp",
    "scheme",
    "solidity",
    "nix",
    "gdscript",
    "glsl",
    "nasm",
    "asm",
    "cmake",
    "nginx",
    "http",
    "promql",
    "jq",
    "json5",
    "properties",
    "latex",
    "mermaid",
    "plantuml",
];

/// Normalize only the lookup key, never the caller's persisted language attribute.
pub fn lookup_language(language: &str) -> String {
    let normalized = language.trim().to_lowercase();
    match normalized.as_str() {
        "plain" | "plaintext" | "text" => "",
        "c++" => "cpp",
        "c#" | "cs" => "csharp",
        "golang" => "go",
        "js" => "javascript",
        "ts" => "typescript",
        "md" => "markdown",
        "obj_c" | "objective-c" => "objectivec",
        "plant-uml" => "plantuml",
        "ps1" => "powershell",
        "py" => "python",
        "rb" => "ruby",
        "shell" | "sh" => "bash",
        "tex" => "latex",
        "vb" => "vbnet",
        "yml" => "yaml",
        _ => return normalized,
    }
    .into()
}
fn grammar(language: &str) -> Option<&'static SyntaxReference> {
    let candidates: &[&str] = match language {
        "csharp" => &["C#", "cs"],
        "objectivec" => &["Objective-C", "m"],
        "bash" => &["Bash", "sh"],
        "jsx" => &["JavaScript (Babel)", "JSX"],
        "tsx" => &["TypeScriptReact", "TypeScript (TSX)", "tsx"],
        "hcl" | "terraform" => &["Terraform", "tf"],
        "nasm" | "asm" => &["Assembly (x86_64)", "asm"],
        "gdscript" => &["GDScript (Godot Engine)", "GDScript", "gd"],
        "fsharp" => &["F#", "fs"],
        "protobuf" => &["Protocol Buffer", "Protocol Buffers", "Protobuf", "proto"],
        "jq" => &["JQ", "jq"],
        // BASIC, VB.NET and Scheme are distinct languages, not aliases for an unrelated grammar.
        _ => &[],
    };
    let set = syntaxes();
    candidates
        .iter()
        .find_map(|candidate| set.find_syntax_by_token(candidate))
        .or_else(|| set.find_syntax_by_token(language))
}
/// Audit every required source language against the actual pinned grammar bundle.
pub fn coverage_report() -> Vec<(&'static str, Coverage)> {
    REQUIRED_LANGUAGES
        .iter()
        .map(|&language| (language, coverage(language)))
        .collect()
}
/// Report actual native grammar availability without attempting to highlight source.
pub fn coverage(language: &str) -> Coverage {
    let language = lookup_language(language);
    if language.is_empty() {
        return Coverage::PlainText;
    }
    if let Some(grammar) = grammar(&language) {
        return Coverage::Highlighted {
            grammar: grammar.name.clone(),
        };
    }
    if REQUIRED_LANGUAGES.contains(&language.as_str()) {
        Coverage::MissingRequiredGrammar { language }
    } else {
        Coverage::UnknownLanguage { language }
    }
}

/// Tokenize the unchanged entire source, carrying multiline grammar state across original lines.
///
/// This CPU operation is suitable for a host worker. Results contain only exact source byte ranges
/// and semantic roles, so a source editor can reuse them with its own typography and palette.
/// Missing required grammars are explicit coverage states and remain release blockers.
pub fn highlight(source: &str, language: &str) -> Result<Highlighted> {
    let language = lookup_language(language);
    let coverage = coverage(&language);
    let Some(grammar) = grammar(&language).filter(|_| !language.is_empty()) else {
        return Ok(Highlighted {
            coverage,
            tokens: if source.is_empty() {
                vec![]
            } else {
                vec![TokenSpan {
                    from: ByteOffset(0),
                    to: ByteOffset(source.len()),
                    kind: TokenKind::Plain,
                }]
            },
        });
    };
    let mut parser = ParseState::new(grammar);
    let mut scopes = ScopeStack::new();
    let mut output = Highlighted {
        coverage,
        tokens: vec![],
    };
    let mut base = 0;
    for line in LinesWithEndings::from(source) {
        let operations = parser
            .parse_line(line, syntaxes())
            .map_err(|_| Error::Parse)?;
        for (range, operation) in ScopeRangeIterator::new(&operations, line) {
            scopes.apply(operation).map_err(|_| Error::Scope)?;
            if range.is_empty() {
                continue;
            }
            let token = TokenSpan {
                from: ByteOffset(base + range.start),
                to: ByteOffset(base + range.end),
                kind: classify(&scopes),
            };
            token.text(source)?;
            if let Some(previous) = output.tokens.last_mut()
                && previous.kind == token.kind
                && previous.to == token.from
            {
                previous.to = token.to;
            } else {
                output.tokens.push(token);
            }
        }
        base += line.len();
    }
    output.validate(source)?;
    Ok(output)
}
fn classify(scopes: &ScopeStack) -> TokenKind {
    for scope in scopes.as_slice().iter().rev() {
        let scope = scope.to_string();
        let kind = if scope.starts_with("comment") {
            TokenKind::Comment
        } else if scope.starts_with("invalid") {
            TokenKind::Invalid
        } else if scope.starts_with("string") || scope.starts_with("constant.character") {
            TokenKind::String
        } else if scope.starts_with("constant.numeric")
            || scope.starts_with("constant.language")
            || scope.starts_with("constant.other")
        {
            TokenKind::Constant
        } else if scope.starts_with("keyword.operator") || scope.starts_with("punctuation") {
            TokenKind::Punctuation
        } else if scope.starts_with("keyword") || scope.starts_with("storage") {
            TokenKind::Keyword
        } else if scope.starts_with("entity.name.function") || scope.starts_with("support.function")
        {
            TokenKind::Function
        } else if scope.starts_with("entity.name.type")
            || scope.starts_with("entity.name.class")
            || scope.starts_with("support.type")
            || scope.starts_with("support.class")
        {
            TokenKind::Type
        } else if scope.starts_with("entity.other.attribute-name")
            || scope.starts_with("support.type.property-name")
        {
            TokenKind::Attribute
        } else {
            continue;
        };
        return kind;
    }
    TokenKind::Plain
}
/// Complete redistribution notices embedded by the pinned grammar bundle.
/// Hosts packaging this module must retain this text alongside dependency licenses.
pub fn acknowledgements_markdown() -> String {
    two_face::acknowledgement::listing().to_md()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_reconstruct_exact_unicode_crlf_tabs_and_trailing_newline() -> Result<()> {
        let source = "// Русский 日本語 🪶\r\nlet 名 = \"e\u{301}\";\n\t名 += 42;\n";
        let result = highlight(source, "rust")?;
        let pieces: Result<Vec<_>> = result
            .tokens
            .iter()
            .map(|token| token.text(source))
            .collect();
        assert_eq!(pieces?.concat(), source);
        Ok(())
    }
    #[test]
    fn rust_has_real_distinct_keyword_string_constant_and_comment_scopes() -> Result<()> {
        let source = "// note\nlet value = \"story\"; let count = 42;";
        let result = highlight(source, "rust")?;
        for kind in [
            TokenKind::Comment,
            TokenKind::Keyword,
            TokenKind::String,
            TokenKind::Constant,
        ] {
            assert!(
                result.tokens.iter().any(|token| token.kind == kind),
                "missing {kind:?}"
            );
        }
        Ok(())
    }
    #[test]
    fn multiline_comment_state_crosses_source_line_boundaries() -> Result<()> {
        let source = "/* first\ninside comment\n*/\nlet a = 1;";
        let result = highlight(source, "rust")?;
        let byte = source.find("inside").ok_or(Error::InvalidTokenRange)?;
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Comment
                    && token.from.0 <= byte
                    && token.to.0 > byte)
        );
        Ok(())
    }
    #[test]
    fn structured_yaml_and_json_are_highlighted_without_programming_language_assumptions()
    -> Result<()> {
        for (language, source) in [
            ("yaml", "story:\n  title: \"Long form\"\n  count: 42\n"),
            ("json", "{\"title\": \"Story\", \"count\": 42}"),
        ] {
            let result = highlight(source, language)?;
            assert!(matches!(result.coverage, Coverage::Highlighted { .. }));
            assert!(
                result
                    .tokens
                    .iter()
                    .any(|token| token.kind != TokenKind::Plain)
            );
            result.validate(source)?;
        }
        Ok(())
    }
    #[test]
    fn explicit_plain_text_and_unknown_imported_language_stay_distinct() -> Result<()> {
        let source = "unchanged <&> 🪶";
        assert_eq!(highlight(source, "")?.coverage, Coverage::PlainText);
        assert!(matches!(
            highlight(source, "custom-author-language")?.coverage,
            Coverage::UnknownLanguage { .. }
        ));
        Ok(())
    }
    #[test]
    fn alias_lookup_does_not_rewrite_callers_original_language_attribute() -> Result<()> {
        let attribute = String::from("  GoLang ");
        assert_eq!(lookup_language(&attribute), "go");
        assert!(matches!(
            highlight("package main", &attribute)?.coverage,
            Coverage::Highlighted { .. }
        ));
        assert_eq!(attribute, "  GoLang ");
        Ok(())
    }
    #[test]
    fn invalid_utf8_token_boundary_is_rejected_before_layout() {
        let token = TokenSpan {
            from: ByteOffset(1),
            to: ByteOffset(2),
            kind: TokenKind::Plain,
        };
        assert!(matches!(token.text("🪶"), Err(Error::InvalidTokenRange)));
    }
    #[test]
    fn every_source_option_is_audited_and_missing_known_grammars_are_never_unknown() {
        let report = coverage_report();
        assert_eq!(report.len(), 77);
        for (language, status) in report {
            assert!(
                !matches!(status, Coverage::UnknownLanguage { .. }),
                "required format not audited: {language}"
            );
        }
    }
    #[test]
    fn all_palette_roles_remain_readable_on_light_dark_and_tinted_surfaces() {
        for (foreground, background) in [
            ([35, 39, 47], [250, 250, 248]),
            ([225, 230, 238], [29, 34, 42]),
            ([231, 220, 210], [47, 35, 30]),
            ([44, 40, 55], [243, 237, 249]),
        ] {
            let palette = Palette::for_surface(foreground, background);
            for role in [
                TokenKind::Plain,
                TokenKind::Comment,
                TokenKind::String,
                TokenKind::Constant,
                TokenKind::Keyword,
                TokenKind::Function,
                TokenKind::Type,
                TokenKind::Punctuation,
                TokenKind::Attribute,
                TokenKind::Invalid,
            ] {
                assert!(
                    contrast(palette.color(role), background) >= 4.5,
                    "unreadable {role:?}"
                );
            }
        }
    }
    #[test]
    fn bundled_acknowledgements_include_real_grammar_license_text() {
        let notices = acknowledgements_markdown();
        assert!(notices.contains("Syntaxes"));
        assert!(notices.contains("Copyright") || notices.contains("copyright"));
    }
}
