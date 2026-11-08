//! Enabled source paste-rule components; source/host policy stays explicit.
use craft_editor_model::is_allowed_link_uri;
use linkify::{LinkFinder, LinkKind};
use std::{collections::HashSet, ops::Range, sync::OnceLock};

/// A recognized source-compatible link with an explicit UTF-8 byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// Exact byte boundaries in the supplied string; caller converts to scalar positions.
    pub bytes: Range<usize>,
    /// Canonical href including original http/mailto default protocol.
    pub href: String,
}
/// Find eligible URL/email spans without opening a URL, network request or filesystem path.
///
/// This is a component checkpoint. The external-paste transform and source corpus qualification
/// determine editor parity; exposing a scanner alone is not authoring/paste acceptance.
pub fn link_spans(text: &str) -> crate::Result<Vec<LinkSpan>> {
    let mut finder = LinkFinder::new();
    finder.url_must_have_scheme(false);
    let masked = mask_unicode_tlds(text);
    let mut result = Vec::new();
    for found in finder.links(&masked) {
        if result.len() >= 100_000 {
            return Err(craft_editor_model::Error::ResourceLimit.into());
        }
        if !text.is_char_boundary(found.start()) || !text.is_char_boundary(found.end()) {
            continue;
        }
        let mut start = found.start();
        let mut end = found.end();
        let original = &text[start..end];
        if found.kind() == &LinkKind::Email
            && start >= 7
            && text.is_char_boundary(start - 7)
            && text[start - 7..start].eq_ignore_ascii_case("mailto:")
            && text[..start - 7]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_alphanumeric())
        {
            start -= 7;
        }
        if found.kind() == &LinkKind::Url {
            while text.as_bytes().get(end) == Some(&b'`') {
                end += 1;
            }
            if text.as_bytes().get(end) == Some(&b'<') && original.contains('/') {
                let token_end = text[end..]
                    .char_indices()
                    .find(|(_, ch)| js_whitespace(*ch))
                    .map_or(text.len(), |(index, _)| end + index);
                let token = text[start..token_end].replace(['<', '>'], "x");
                if let Some(extended) = finder.links(&token).next().filter(|link| link.start() == 0)
                {
                    end = start + extended.end();
                }
            }
        }
        let value = &text[start..end];
        if let Some((scheme, suffix)) = value.split_once(':')
            && valid_scheme(scheme)
            && !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "ftp" | "ftps" | "mailto" | "file"
            )
        {
            // The source scanner recognizes a bare URL/email following an unregistered prefix,
            // rather than admitting an arbitrary new protocol into its canonical mark.
            let offset = start + scheme.len() + 1;
            let prefix = suffix.len() - suffix.trim_start_matches('/').len();
            for nested in finder.links(&suffix[prefix..]) {
                if let Some(href) = href(nested.as_str(), nested.kind()) {
                    result.push(LinkSpan {
                        bytes: offset + prefix + nested.start()..offset + prefix + nested.end(),
                        href,
                    });
                }
            }
        } else if let Some(href) = href(value, found.kind()) {
            result.push(LinkSpan {
                bytes: start..end,
                href,
            });
        }
    }
    Ok(result)
}
fn mask_unicode_tlds(text: &str) -> String {
    // linkify's bare-host fast path requires an ASCII TLD. Replace only a frozen, recognized
    // Unicode TLD label with same-byte-length ASCII for scanning, then use the exact original
    // bytes for every emitted range/href. No normalization changes the author's canonical text.
    let mut masked = text.to_owned();
    for (dot, ch) in text.char_indices() {
        if ch != '.' {
            continue;
        }
        let start = dot + 1;
        let end = text[start..]
            .char_indices()
            .find(|(_, ch)| ch.is_ascii_punctuation() || js_whitespace(*ch))
            .map_or(text.len(), |(offset, _)| start + offset);
        let label = &text[start..end];
        if !label.is_ascii() && known_tld(&format!("x.{label}")) {
            masked.replace_range(start..end, &"a".repeat(end - start));
        }
    }
    // Same-byte-length replacements preserve every original subsequent boundary.
    masked
}
fn valid_scheme(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"+.-".contains(&ch))
}
fn known_tld(host: &str) -> bool {
    static TLDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let tlds = TLDS.get_or_init(|| {
        include_str!("../../third_party/linkifyjs/tlds-4.3.3.txt")
            .lines()
            .collect()
    });
    host.rsplit_once('.')
        .is_some_and(|(_, last)| tlds.contains(last.to_lowercase().as_str()))
}
fn href(value: &str, kind: &LinkKind) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let explicit = ["http://", "https://", "ftp://", "ftps://"]
        .iter()
        .find(|prefix| lower.starts_with(**prefix));
    let href = if let Some(prefix) = explicit {
        // The installed source scanner does not recognize bracketed IPv6 authority literals.
        if value[prefix.len()..].starts_with('[') {
            return None;
        }
        value.to_owned()
    } else if lower.starts_with("mailto:") {
        let mail = &value[7..];
        if !mail
            .rsplit_once('@')
            .is_some_and(|(_, domain)| known_tld(domain))
        {
            return None;
        }
        value.to_owned()
    } else if kind == &LinkKind::Email {
        if !value
            .rsplit_once('@')
            .is_some_and(|(_, domain)| known_tld(domain))
        {
            return None;
        }
        format!("mailto:{value}")
    } else {
        if lower.starts_with("file:") {
            return None;
        }
        let authority = value.split(['/', '?', '#']).next()?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        if !known_tld(host) {
            return None;
        }
        format!("http://{value}")
    };
    is_allowed_link_uri(&href).then_some(href)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls_emails_default_protocol_and_shared_uri_policy_match_enabled_source_basics()
    -> crate::Result<()> {
        for (text, expected) in [
            ("example.com", Some("http://example.com")),
            ("a@example.com", Some("mailto:a@example.com")),
            ("https://example.com", Some("https://example.com")),
            ("ftp://example.com/a", Some("ftp://example.com/a")),
            ("example.invalid", None),
            ("a@example.invalid", None),
            ("127.0.0.1", None),
            ("localhost", None),
            ("javascript:bad", None),
            ("file:///etc/private", None),
        ] {
            let found = link_spans(text)?;
            assert_eq!(found.len(), usize::from(expected.is_some()), "{text}");
            if let Some(expected) = expected {
                assert_eq!(found[0].href, expected, "{text}");
                assert_eq!(found[0].bytes, 0..text.len());
            }
        }
        Ok(())
    }
    #[test]
    fn link_boundaries_are_utf8_bytes_and_surrounding_unicode_stays_outside_href()
    -> crate::Result<()> {
        let text = "先😀 https://example.com/后 — a@example.com";
        let links = link_spans(text)?;
        assert_eq!(links.len(), 2);
        assert_eq!(&text[links[0].bytes.clone()], "https://example.com/后");
        assert_eq!(&text[links[1].bytes.clone()], "a@example.com");
        assert_eq!(text[..links[0].bytes.start].chars().count(), 3);
        Ok(())
    }
}

use super::RichSlice;
use crate::Result;
use craft_editor_model::{Mark, Node};
use serde_json::{Map, Value, json};

#[derive(Clone, Copy)]
enum Rule {
    Code,
    BoldStar,
    BoldUnder,
    ItalicStar,
    ItalicUnder,
    Strike,
    Link,
}
impl Rule {
    fn marker(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Code => Some(("`", "code")),
            Self::BoldStar => Some(("**", "bold")),
            Self::BoldUnder => Some(("__", "bold")),
            Self::ItalicStar => Some(("*", "italic")),
            Self::ItalicUnder => Some(("_", "italic")),
            Self::Strike => Some(("~~", "strike")),
            Self::Link => None,
        }
    }
}
struct Run {
    node: Node,
    bytes: Range<usize>,
}
struct Replacement {
    bytes: Range<usize>,
    mark: Option<Mark>,
    delete: bool,
}
fn resource_limit() -> crate::Error {
    craft_editor_model::Error::ResourceLimit.into()
}
struct Budget {
    nodes: usize,
    bytes: usize,
}
impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.nodes = self.nodes.checked_sub(1).ok_or_else(resource_limit)?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(resource_limit)?;
        Ok(())
    }
}
pub(super) fn serialized_bytes<T: serde::Serialize + ?Sized>(value: &T) -> Result<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("clipboard_resource_limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(craft_editor_model::Error::from)?;
    Ok(counter.0)
}
fn metadata_bytes(node: &Node) -> Result<usize> {
    [
        serialized_bytes(&node.attrs)?,
        serialized_bytes(&node.marks)?,
        serialized_bytes(&node.extra)?,
        node.node_type.len(),
    ]
    .into_iter()
    .try_fold(64usize, usize::checked_add)
    .ok_or_else(resource_limit)
}
fn text_bytes(text: &str) -> Result<usize> {
    serialized_bytes(text)
}
fn preflight(nodes: &[Node], budget: &mut Budget, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(resource_limit());
    }
    for node in nodes {
        let cost = metadata_bytes(node)?
            .checked_add(node.text.as_deref().map_or(Ok(0), text_bytes)?)
            .ok_or_else(resource_limit)?;
        budget.charge(cost)?;
        preflight(node.children(), budget, depth + 1)?;
    }
    Ok(())
}
fn js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
fn flatten(nodes: Vec<Node>) -> (String, Vec<Run>) {
    let mut text = String::new();
    let mut runs = Vec::with_capacity(nodes.len());
    for node in nodes {
        let start = text.len();
        if node.node_type == "text" {
            text.push_str(node.text.as_deref().unwrap_or_default());
        } else {
            text.push('\u{fffc}');
        }
        runs.push(Run {
            node,
            bytes: start..text.len(),
        });
    }
    (text, runs)
}
fn overlaps_code(runs: &[Run], cursor: &mut usize, range: &Range<usize>) -> bool {
    while runs
        .get(*cursor)
        .is_some_and(|run| run.bytes.end <= range.start)
    {
        *cursor += 1;
    }
    runs[*cursor..]
        .iter()
        .take_while(|run| run.bytes.start < range.end)
        .any(|run| {
            run.node
                .marks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|mark| mark.mark_type == "code")
        })
}
fn replacements(text: &str, runs: &[Run], rule: Rule) -> Result<Vec<Replacement>> {
    let mut output = Vec::new();
    let mut run_cursor = 0;
    let Some((marker, kind)) = rule.marker() else {
        for link in link_spans(text)? {
            if overlaps_code(runs, &mut run_cursor, &link.bytes) {
                continue;
            }
            let mut mark = Mark::new("link");
            mark.attrs = Some(Map::from_iter([
                ("href".into(), json!(link.href)),
                ("target".into(), json!("_blank")),
                ("rel".into(), json!("noopener noreferrer")),
                ("class".into(), Value::Null),
                ("title".into(), Value::Null),
            ]));
            output.push(Replacement {
                bytes: link.bytes,
                mark: Some(mark),
                delete: false,
            });
        }
        return Ok(output);
    };
    let byte = marker.as_bytes()[0];
    let width = marker.len();
    let mut offset = 0;
    while offset < text.len() {
        if text.as_bytes()[offset] != byte {
            offset += 1;
            continue;
        }
        let before = text[..offset].chars().next_back();
        let allowed = if matches!(rule, Rule::Code) {
            before != Some('`')
        } else {
            before.is_none_or(js_whitespace)
        };
        if !allowed || !text[offset..].starts_with(marker) {
            offset += 1;
            continue;
        }
        let body_start = offset + width;
        let Some(relative) = text[body_start..].bytes().position(|ch| ch == byte) else {
            break;
        };
        let body_end = body_start + relative;
        let end = body_end + width;
        if body_start == body_end
            || end > text.len()
            || !text[body_end..].starts_with(marker)
            || (!matches!(rule, Rule::Code)
                && text[body_start..body_end].chars().all(js_whitespace))
            || (matches!(rule, Rule::Code) && text.as_bytes().get(end) == Some(&b'`'))
        {
            offset += 1;
            continue;
        }
        let full = offset..end;
        if !matches!(rule, Rule::Code) && overlaps_code(runs, &mut run_cursor, &full) {
            offset = end;
            continue;
        }
        if output.len() >= 300_000 {
            return Err(resource_limit());
        }
        output.push(Replacement {
            bytes: offset..body_start,
            mark: None,
            delete: true,
        });
        output.push(Replacement {
            bytes: body_start..body_end,
            mark: Some(Mark::new(kind)),
            delete: false,
        });
        output.push(Replacement {
            bytes: body_end..end,
            mark: None,
            delete: true,
        });
        offset = end;
    }
    Ok(output)
}
fn apply_mark(node: &mut Node, mark: &Mark) {
    let marks = node.marks.get_or_insert_default();
    if mark.mark_type == "code" {
        marks.retain(|existing| existing.mark_type == "code");
    }
    if let Some(existing) = marks
        .iter_mut()
        .find(|existing| existing.mark_type == mark.mark_type)
    {
        // Existing opaque attributes/fields remain authoritative. A new link changes only href;
        // other source attributes such as title/target and explicit null presence stay intact.
        if mark.mark_type == "link"
            && let Some(href) = mark.attrs.as_ref().and_then(|attrs| attrs.get("href"))
        {
            existing
                .attrs
                .get_or_insert_default()
                .insert("href".into(), href.clone());
        }
    } else {
        marks.push(mark.clone());
    }
}
fn transform_inline(nodes: Vec<Node>, rule: Rule, remaining: &mut Budget) -> Result<Vec<Node>> {
    let (text, runs) = flatten(nodes);
    let replacements = replacements(&text, &runs, rule)?;
    let mut output = Vec::new();
    let mut event = 0;
    for run in runs {
        let metadata = metadata_bytes(&run.node)?;
        let mut template = run.node;
        // Clone only the envelope for each output fragment, never the full original text again.
        template.text = None;
        let mut offset = run.bytes.start;
        while offset < run.bytes.end {
            while replacements
                .get(event)
                .is_some_and(|edit| edit.bytes.end <= offset)
            {
                event += 1;
            }
            let edit = replacements
                .get(event)
                .filter(|edit| edit.bytes.start < run.bytes.end);
            let end = edit.map_or(run.bytes.end, |edit| {
                if offset < edit.bytes.start {
                    edit.bytes.start.min(run.bytes.end)
                } else {
                    edit.bytes.end.min(run.bytes.end)
                }
            });
            let active = edit.filter(|edit| offset >= edit.bytes.start);
            if !active.is_some_and(|edit| edit.delete) {
                let extra_mark = active
                    .and_then(|edit| edit.mark.as_ref())
                    .map(|mark| serde_json::to_vec(mark).map(|bytes| bytes.len()))
                    .transpose()
                    .map_err(craft_editor_model::Error::from)?
                    .unwrap_or_default();
                let text_cost = if template.node_type == "text" {
                    text_bytes(&text[offset..end])?
                } else {
                    0
                };
                let cost = metadata
                    .checked_add(extra_mark)
                    .and_then(|cost| cost.checked_add(text_cost))
                    .ok_or_else(resource_limit)?;
                // Count the existing envelope before cloning it into another fragment. Oversized
                // opaque metadata expansion fails while the original slice is still untouched.
                remaining.charge(cost)?;
                let mut node = template.clone();
                if node.node_type == "text" {
                    node.text = Some(text[offset..end].to_owned());
                }
                if let Some(mark) = active.and_then(|edit| edit.mark.as_ref()) {
                    apply_mark(&mut node, mark);
                }
                output.push(node);
            }
            offset = end;
        }
    }
    Ok(output)
}
fn walk(
    nodes: &mut [Node],
    rule: Rule,
    remaining: &mut Budget,
    depth: usize,
    skip: bool,
) -> Result<()> {
    if depth > 64 {
        return Err(resource_limit());
    }
    for node in nodes {
        let mut cost = metadata_bytes(node)?;
        if let Some(text) = &node.text {
            cost = cost
                .checked_add(text_bytes(text)?)
                .ok_or_else(resource_limit)?;
        }
        remaining.charge(cost)?;
        let skip = skip || node.node_type == "codeBlock";
        if !skip && matches!(node.node_type.as_str(), "paragraph" | "heading") {
            if let Some(children) = node.content.take() {
                node.content = Some(transform_inline(children, rule, remaining)?);
            }
        } else if let Some(children) = &mut node.content {
            walk(children, rule, remaining, depth + 1, skip)?;
        }
    }
    Ok(())
}
/// Transform only eligible external rich input, retaining canonical wrappers and opaque fields.
/// Private/internal clipboard representations must bypass this entry point at the host boundary.
pub fn transform_external(slice: &mut RichSlice, max_output_bytes: usize) -> Result<()> {
    preflight(
        &slice.content,
        &mut Budget {
            nodes: 100_000,
            bytes: max_output_bytes,
        },
        0,
    )?;
    let mut candidate = slice.clone();
    for rule in [
        Rule::Code,
        Rule::BoldStar,
        Rule::BoldUnder,
        Rule::ItalicStar,
        Rule::ItalicUnder,
        Rule::Strike,
        Rule::Link,
    ] {
        // This is the existing canonical model node bound, not a smaller feature quota.
        let mut remaining = Budget {
            nodes: 100_000,
            bytes: max_output_bytes,
        };
        walk(&mut candidate.content, rule, &mut remaining, 0, false)?;
    }
    *slice = candidate;
    Ok(())
}
/// Build external plain paragraphs from an exact inherited text/mark envelope before applying
/// enabled source rules. Paragraph breaks stay explicit; source code blocks use a separate path.
pub fn plain_slice(text: &str, template: &Node, max_output_bytes: usize) -> Result<RichSlice> {
    let metadata = metadata_bytes(template)?;
    let mut budget = Budget {
        nodes: 100_000,
        bytes: max_output_bytes,
    };
    for line in text.split('\n') {
        budget.charge(96)?;
        if !line.is_empty() {
            budget.charge(
                metadata
                    .checked_add(text_bytes(line)?)
                    .ok_or_else(resource_limit)?,
            )?;
        }
    }
    let envelope = template.text_envelope()?;
    let mut slice = RichSlice {
        content: text
            .split('\n')
            .map(|line| {
                let mut leaf = envelope.clone();
                leaf.text = Some(line.into());
                Node::block(
                    "paragraph",
                    Map::new(),
                    if line.is_empty() { vec![] } else { vec![leaf] },
                )
            })
            .collect(),
        open_start: 1,
        open_end: 1,
    };
    transform_external(&mut slice, max_output_bytes)?;
    Ok(slice)
}
