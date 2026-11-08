//! Source-backed complete-fence/formula paste and non-sentinel plain fallbacks.
use super::RichSlice;
use crate::Result;
use craft_editor_model::Node;
use serde_json::{Map, Value};

pub(super) fn special_slice(value: &str) -> Result<Option<RichSlice>> {
    if let Some((language, text)) = fenced_code(value) {
        let attrs = if language.is_empty() {
            Map::new()
        } else {
            Map::from_iter([("language".into(), Value::String(language))])
        };
        let children = if text.is_empty() {
            vec![]
        } else {
            vec![Node::text(text)]
        };
        return Ok(Some(RichSlice {
            content: vec![Node::block("codeBlock", attrs, children)],
            open_start: 0,
            open_end: 0,
        }));
    }
    let source = value.trim();
    let formula = if let Some(value) = source
        .strip_prefix("$$")
        .and_then(|value| value.strip_suffix("$$"))
    {
        (!value.trim().is_empty()).then_some((true, value.trim()))
    } else if let Some(value) = source
        .strip_prefix('$')
        .and_then(|value| value.strip_suffix('$'))
    {
        (!value.trim().is_empty() && !value.contains(['$', '\r', '\n']))
            .then_some((false, value.trim()))
    } else {
        None
    };
    if let Some((display, latex)) = formula {
        return Ok(Some(RichSlice {
            content: vec![Node::block(
                if display { "mathBlock" } else { "inlineMath" },
                Map::from_iter([
                    ("latex".into(), Value::String(latex.into())),
                    ("description".into(), Value::String(String::new())),
                ]),
                vec![],
            )],
            open_start: 0,
            open_end: 0,
        }));
    }
    Ok(None)
}
fn fenced_code(value: &str) -> Option<(String, String)> {
    let first = value.find('\n')?;
    let opening = value[..first].strip_suffix('\r').unwrap_or(&value[..first]);
    let fence = opening.chars().next()?;
    if !matches!(fence, '`' | '~') {
        return None;
    }
    let count = opening.chars().take_while(|ch| *ch == fence).count();
    if count < 3 {
        return None;
    }
    let info = opening[count..].trim();
    let language = info.split_whitespace().next().unwrap_or_default();
    if !language.is_empty()
        && (language.len() > 40
            || !language
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || b"_+#.-".contains(&ch)))
    {
        return None;
    }
    let mut remainder = &value[first + 1..];
    if let Some(trimmed) = remainder
        .strip_suffix("\r\n")
        .or_else(|| remainder.strip_suffix('\n'))
    {
        remainder = trimmed;
    }
    let (text, closing) = if let Some(last) = remainder.rfind('\n') {
        (
            remainder[..last]
                .strip_suffix('\r')
                .unwrap_or(&remainder[..last]),
            &remainder[last + 1..],
        )
    } else {
        ("", remainder)
    };
    let indent = closing
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count();
    if indent > 3 {
        return None;
    }
    let closing = closing[indent..].trim_end();
    if closing.len() < count || !closing.chars().all(|ch| ch == fence) {
        return None;
    }
    Some((language.to_ascii_lowercase(), text.into()))
}
pub(super) fn export(nodes: &[Node]) -> String {
    nodes.iter().map(node_text).collect::<Vec<_>>().join("\n")
}
fn attr<'a>(node: &'a Node, key: &str) -> &'a str {
    node.attrs
        .as_ref()
        .and_then(|attrs| attrs.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
}
fn inline(nodes: &[Node]) -> String {
    nodes.iter().map(node_text).collect()
}
fn node_text(node: &Node) -> String {
    match node.node_type.as_str() {
        "text" => node.text.clone().unwrap_or_default(),
        "hardBreak" => "\n".into(),
        "inlineMath" => format!("${}$", attr(node, "latex")),
        "mathBlock" => format!("$$\n{}\n$$", attr(node, "latex")),
        "mermaidBlock" => {
            let code = attr(node, "code");
            let ticks = fence_for(code);
            let caption = attr(node, "caption");
            format!(
                "{ticks}mermaid\n{code}\n{ticks}{}",
                if caption.is_empty() {
                    String::new()
                } else {
                    format!("\n{caption}")
                }
            )
        }
        "imageBlock" => {
            let alt = attr(node, "alt");
            let caption = attr(node, "caption");
            let label = if alt.is_empty() { caption } else { alt };
            format!(
                "![{label}]({}){}",
                attr(node, "src"),
                if caption.is_empty() || caption == label {
                    String::new()
                } else {
                    format!("\n{caption}")
                }
            )
        }
        "codeBlock" => {
            let text = node.inline_text();
            let fence = fence_for(&text);
            format!("{fence}{}\n{text}\n{fence}", attr(node, "language"))
        }
        "horizontalRule" => "---".into(),
        "sceneBreak" => "* * *".into(),
        "paragraph" | "heading" => inline(node.children()),
        "tableRow" => node
            .children()
            .iter()
            .map(|node| export(node.children()))
            .collect::<Vec<_>>()
            .join("\t"),
        _ => export(node.children()),
    }
}
fn fence_for(text: &str) -> String {
    let mut longest: usize = 0;
    let mut run: usize = 0;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(longest.saturating_add(1).max(3))
}
