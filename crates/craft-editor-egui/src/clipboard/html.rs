//! Source-backed HTML5 import: retain supported semantics, discard active/external behavior.
use super::RichSlice;
use crate::{Error, Result};
use craft_editor_model::{Mark, Node};
use ego_tree::NodeRef;
use scraper::{Html, Node as HtmlNode};
use serde_json::{Map, Value};

fn attr<'a>(node: &'a Node, key: &str) -> &'a str {
    node.attrs
        .as_ref()
        .and_then(|attrs| attrs.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
}
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub(super) fn export(nodes: &[Node]) -> String {
    nodes.iter().map(export_node).collect()
}
fn export_node(node: &Node) -> String {
    if node.node_type == "text" {
        let mut text = escaped(node.text.as_deref().unwrap_or_default());
        for mark in node.marks.as_deref().unwrap_or_default().iter().rev() {
            let tag = match mark.mark_type.as_str() {
                "bold" => "strong",
                "italic" => "em",
                "underline" => "u",
                "strike" => "s",
                "code" => "code",
                "link" => "a",
                _ => continue,
            };
            let attrs = if tag == "a" {
                format!(
                    " href=\"{}\"",
                    escaped(
                        mark.attrs
                            .as_ref()
                            .and_then(|attrs| attrs.get("href"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    )
                )
            } else {
                String::new()
            };
            text = format!("<{tag}{attrs}>{text}</{tag}>");
        }
        return text;
    }
    let provenance = {
        let mut value = String::new();
        if let Some(id) = node.block_id() {
            value.push_str(&format!(" data-block-id=\"{}\"", escaped(id)));
        }
        if let Some(refs) = node
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("sourceRefs"))
            .and_then(Value::as_array)
        {
            let refs = refs
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",");
            if !refs.is_empty() {
                value.push_str(&format!(" data-source-refs=\"{}\"", escaped(&refs)));
            }
        }
        value
    };
    let body = || export(node.children());
    match node.node_type.as_str() {
        "hardBreak" => "<br>".into(),
        "horizontalRule" => "<hr>".into(),
        "sceneBreak" => "<div data-scene-break=\"\">* * *</div>".into(),
        "paragraph" => format!("<p{provenance}>{}</p>", body()),
        "heading" => {
            let level = node
                .attrs
                .as_ref()
                .and_then(|attrs| attrs.get("level"))
                .and_then(Value::as_u64)
                .unwrap_or(1);
            format!("<h{level}{provenance}>{}</h{level}>", body())
        }
        "blockquote" => format!("<blockquote{provenance}>{}</blockquote>", body()),
        "bulletList" => format!("<ul>{}</ul>", body()),
        "orderedList" => {
            let start = node
                .attrs
                .as_ref()
                .and_then(|attrs| attrs.get("start"))
                .and_then(Value::as_i64)
                .unwrap_or(1);
            format!("<ol start=\"{start}\">{}</ol>", body())
        }
        "listItem" => format!("<li>{}</li>", body()),
        "callout" => format!(
            "<aside{provenance} data-callout-kind=\"{}\">{}</aside>",
            escaped(attr(node, "kind")),
            body()
        ),
        "codeBlock" => format!(
            "<pre{provenance}><code class=\"language-{}\">{}</code></pre>",
            escaped(attr(node, "language")),
            escaped(&node.inline_text())
        ),
        "table" => format!("<table{provenance}><tbody>{}</tbody></table>", body()),
        "tableRow" => format!("<tr>{}</tr>", body()),
        "tableCell" | "tableHeader" => {
            let tag = if node.node_type == "tableCell" {
                "td"
            } else {
                "th"
            };
            let mut attrs = String::new();
            for name in ["colspan", "rowspan"] {
                if let Some(value) = node
                    .attrs
                    .as_ref()
                    .and_then(|attrs| attrs.get(name))
                    .and_then(Value::as_u64)
                {
                    attrs.push_str(&format!(" {name}=\"{value}\""));
                }
            }
            if let Some(widths) = node
                .attrs
                .as_ref()
                .and_then(|attrs| attrs.get("colwidth"))
                .and_then(Value::as_array)
            {
                attrs.push_str(&format!(
                    " data-colwidth=\"{}\"",
                    widths
                        .iter()
                        .filter_map(Value::as_u64)
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            format!("<{tag}{attrs}>{}</{tag}>", body())
        }
        "imageBlock" => format!(
            "<figure{provenance} data-image-block=\"\" data-asset-id=\"{}\"><img src=\"{}\" alt=\"{}\"{}><figcaption>{}</figcaption></figure>",
            escaped(attr(node, "assetId")),
            escaped(attr(node, "src")),
            escaped(attr(node, "alt")),
            ["width", "height"]
                .iter()
                .filter_map(|name| node
                    .attrs
                    .as_ref()
                    .and_then(|attrs| attrs.get(*name))
                    .and_then(Value::as_u64)
                    .map(|value| format!(" {name}=\"{value}\"")))
                .collect::<String>(),
            escaped(attr(node, "caption"))
        ),
        "mermaidBlock" => format!(
            "<figure{provenance} data-mermaid-block=\"\"><pre data-mermaid-source=\"\">{}</pre><figcaption>{}</figcaption></figure>",
            escaped(attr(node, "code")),
            escaped(attr(node, "caption"))
        ),
        "inlineMath" => format!(
            "<span data-inline-math=\"\" data-latex=\"{}\" data-description=\"{}\">{}</span>",
            escaped(attr(node, "latex")),
            escaped(attr(node, "description")),
            escaped(attr(node, "latex"))
        ),
        "mathBlock" => format!(
            "<figure{provenance} data-math-block=\"\" data-latex=\"{}\" data-description=\"{}\"><pre data-math-source=\"\">{}</pre></figure>",
            escaped(attr(node, "latex")),
            escaped(attr(node, "description")),
            escaped(attr(node, "latex"))
        ),
        _ => body(),
    }
}

pub(super) fn parse(value: &str) -> Result<RichSlice> {
    let document = Html::parse_fragment(value);
    let mut budget = 100_000usize;
    let nodes = parse_children(document.tree.root(), &[], 0, &mut budget)?;
    let mut content = group_blocks(nodes);
    fn fresh_html_ids(nodes: &mut [Node]) {
        for node in nodes {
            if let Some(attrs) = &mut node.attrs {
                attrs.remove("blockId");
            }
            if let Some(children) = &mut node.content {
                fresh_html_ids(children);
            }
        }
    }
    fresh_html_ids(&mut content);
    let inline_edges = content.first().is_some_and(|node| {
        matches!(
            node.node_type.as_str(),
            "paragraph" | "heading" | "codeBlock"
        )
    }) && content.last().is_some_and(|node| {
        matches!(
            node.node_type.as_str(),
            "paragraph" | "heading" | "codeBlock"
        )
    });
    let open_start = usize::from(inline_edges);
    let open_end = usize::from(inline_edges);
    // Validate untrusted fragment fields before it reaches the insertion planner. Whole candidate
    // validation still checks actual grammar/table grid after fitting open edges.
    let content = Node::decode_open_fragment(
        serde_json::to_value(content).map_err(craft_editor_model::Error::from)?,
    )?;
    Ok(RichSlice {
        content,
        open_start,
        open_end,
    })
}
fn parse_children(
    parent: NodeRef<'_, HtmlNode>,
    marks: &[Mark],
    depth: usize,
    budget: &mut usize,
) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for child in parent.children() {
        nodes.extend(parse_node(child, marks, depth + 1, budget)?);
    }
    Ok(nodes)
}
fn parse_node(
    node: NodeRef<'_, HtmlNode>,
    marks: &[Mark],
    depth: usize,
    budget: &mut usize,
) -> Result<Vec<Node>> {
    if depth > 64 || *budget == 0 {
        return Err(craft_editor_model::Error::ResourceLimit.into());
    }
    *budget -= 1;
    if let HtmlNode::Text(text) = node.value() {
        let text = text.text.to_string();
        if text.is_empty() {
            return Ok(vec![]);
        }
        let mut leaf = Node::text(text);
        if !marks.is_empty() {
            leaf.marks = Some(marks.to_vec());
        }
        return Ok(vec![leaf]);
    }
    let Some(element) = node.value().as_element() else {
        return parse_children(node, marks, depth, budget);
    };
    let tag = element.name();
    if matches!(
        tag,
        "button"
            | "embed"
            | "form"
            | "iframe"
            | "input"
            | "link"
            | "math"
            | "meta"
            | "object"
            | "script"
            | "style"
            | "svg"
            | "textarea"
    ) {
        return Ok(vec![]);
    }
    let mut marks = marks.to_vec();
    let mark_kind = match tag {
        "strong" | "b" => Some("bold"),
        "em" | "i" => Some("italic"),
        "u" => Some("underline"),
        "s" | "strike" | "del" => Some("strike"),
        "code" => Some("code"),
        _ => None,
    };
    if let Some(kind) = mark_kind {
        put_mark(&mut marks, Mark::new(kind));
    }
    if let Some(style) = element.attr("style") {
        for (name, value) in style.split(';').filter_map(|decl| decl.split_once(':')) {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_ascii_lowercase();
            if name == "font-weight"
                && (value == "bold" || matches!(value.as_str(), "600" | "700" | "800" | "900"))
            {
                put_mark(&mut marks, Mark::new("bold"));
            }
            if name == "font-style" && value == "italic" {
                put_mark(&mut marks, Mark::new("italic"));
            }
            if matches!(name.as_str(), "text-decoration" | "text-decoration-line") {
                if value.contains("underline") {
                    put_mark(&mut marks, Mark::new("underline"));
                }
                if value.contains("line-through") {
                    put_mark(&mut marks, Mark::new("strike"));
                }
            }
        }
    }
    if tag == "a"
        && let Some(href) = element.attr("href").filter(|value| safe_html_href(value))
    {
        let mut link = Mark::new("link");
        link.attrs = Some(Map::from_iter([(
            "href".into(),
            Value::String(href.into()),
        )]));
        put_mark(&mut marks, link);
    }
    let mut attrs = Map::new();
    if let Some(id) = element
        .attr("data-block-id")
        .filter(|value| safe_provenance(value))
    {
        attrs.insert("blockId".into(), Value::String(id.into()));
    }
    if let Some(refs) = element
        .attr("data-source-refs")
        .filter(|value| safe_provenance(value))
    {
        attrs.insert(
            "sourceRefs".into(),
            Value::Array(
                refs.split(',')
                    .filter(|value| !value.is_empty())
                    .map(|value| Value::String(value.into()))
                    .collect(),
            ),
        );
    }
    if tag == "br" {
        return Ok(vec![Node::block("hardBreak", Map::new(), vec![])]);
    }
    if tag == "hr" {
        return Ok(vec![Node::block("horizontalRule", Map::new(), vec![])]);
    }
    if tag == "div" && element.attr("data-scene-break").is_some() {
        return Ok(vec![Node::block("sceneBreak", Map::new(), vec![])]);
    }
    // Unlike the source sanitizer's accidental math-attribute stripping, preserve the source's
    // own serialized formula elements, while ignoring rendered MathML/SVG and active content.
    if (tag == "span" && element.attr("data-inline-math").is_some())
        || (tag == "figure" && element.attr("data-math-block").is_some())
    {
        let latex = element
            .attr("data-latex")
            .map(str::to_owned)
            .unwrap_or_else(|| text_content(node));
        attrs.insert("latex".into(), Value::String(latex));
        attrs.insert(
            "description".into(),
            Value::String(element.attr("data-description").unwrap_or_default().into()),
        );
        return Ok(vec![Node::block(
            if tag == "span" {
                "inlineMath"
            } else {
                "mathBlock"
            },
            attrs,
            vec![],
        )]);
    }
    if tag == "figure" && element.attr("data-mermaid-block").is_some() {
        let source = node.descendants().find(|child| {
            child.value().as_element().is_some_and(|element| {
                element.name() == "pre" && element.attr("data-mermaid-source").is_some()
            })
        });
        let caption = node.descendants().find(|child| {
            child
                .value()
                .as_element()
                .is_some_and(|element| element.name() == "figcaption")
        });
        attrs.insert(
            "code".into(),
            Value::String(source.map(text_content).unwrap_or_default()),
        );
        attrs.insert(
            "caption".into(),
            Value::String(caption.map(text_content).unwrap_or_default()),
        );
        return Ok(vec![Node::block("mermaidBlock", attrs, vec![])]);
    }
    if tag == "figure" && element.attr("data-image-block").is_some() {
        let image = node.descendants().find(|child| {
            child
                .value()
                .as_element()
                .is_some_and(|element| element.name() == "img")
        });
        if let Some(image) = image {
            if let Some(mut image) = parse_image(image, attrs) {
                let caption = node.descendants().find(|child| {
                    child
                        .value()
                        .as_element()
                        .is_some_and(|element| element.name() == "figcaption")
                });
                image.attrs.get_or_insert_default().insert(
                    "caption".into(),
                    Value::String(caption.map(text_content).unwrap_or_default()),
                );
                if let Some(id) = element
                    .attr("data-asset-id")
                    .filter(|id| safe_identifier(id))
                {
                    image
                        .attrs
                        .get_or_insert_default()
                        .insert("assetId".into(), Value::String(id.into()));
                }
                return Ok(vec![image]);
            }
        }
        return Ok(vec![]);
    }
    if tag == "img" {
        return Ok(parse_image(node, attrs).into_iter().collect());
    }
    if tag == "pre" {
        let language = node.descendants().find_map(|child| {
            child
                .value()
                .as_element()
                .filter(|el| el.name() == "code")
                .and_then(|el| el.attr("class"))
                .and_then(|class| class.strip_prefix("language-"))
                .filter(|lang| {
                    !lang.is_empty()
                        && lang.len() <= 40
                        && lang
                            .bytes()
                            .all(|ch| ch.is_ascii_alphanumeric() || b"_+#.-".contains(&ch))
                })
        });
        if let Some(language) = language {
            attrs.insert(
                "language".into(),
                Value::String(language.to_ascii_lowercase()),
            );
        }
        let text = text_content(node);
        return Ok(vec![Node::block(
            "codeBlock",
            attrs,
            if text.is_empty() {
                vec![]
            } else {
                vec![Node::text(text)]
            },
        )]);
    }
    let children = parse_children(node, &marks, depth, budget)?;
    let kind = match tag {
        "p" => Some("paragraph"),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            attrs.insert("level".into(), Value::from(tag.as_bytes()[1] - b'0'));
            Some("heading")
        }
        "blockquote" => Some("blockquote"),
        "ul" => Some("bulletList"),
        "ol" => Some("orderedList"),
        "li" => Some("listItem"),
        "table" => Some("table"),
        "tr" => Some("tableRow"),
        "td" | "th" => Some(if tag == "td" {
            "tableCell"
        } else {
            "tableHeader"
        }),
        "aside" => {
            let kind = element
                .attr("data-callout-kind")
                .filter(|value| matches!(*value, "info" | "idea" | "warning" | "todo"))
                .unwrap_or("info");
            attrs.insert("kind".into(), Value::String(kind.into()));
            Some("callout")
        }
        "div" if !children.iter().any(is_block) => Some("paragraph"),
        _ => None,
    };
    if matches!(tag, "td" | "th") {
        for name in ["colspan", "rowspan"] {
            if let Some(number) = element
                .attr(name)
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|number| (1..=100).contains(number))
            {
                attrs.insert(name.into(), Value::from(number));
            }
        }
        if let Some(widths) = element.attr("data-colwidth").filter(|value| {
            value.len() <= 1000
                && !value.is_empty()
                && value
                    .split(',')
                    .all(|width| !width.is_empty() && width.bytes().all(|ch| ch.is_ascii_digit()))
        }) {
            let values = widths
                .split(',')
                .map(|value| value.parse::<u64>())
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::InvalidSelection)?;
            attrs.insert(
                "colwidth".into(),
                Value::Array(values.into_iter().map(Value::from).collect()),
            );
        }
    }
    let Some(kind) = kind else {
        return Ok(children);
    };
    if matches!(kind, "paragraph" | "heading") && children.iter().any(is_block) {
        let mut blocks = group_blocks(children);
        for node in &mut blocks {
            if node.node_type == "paragraph" {
                node.node_type = kind.into();
                node.attrs = Some(attrs.clone());
            }
        }
        return Ok(blocks);
    }
    let content = if matches!(kind, "table" | "tableRow" | "bulletList" | "orderedList") {
        children.into_iter().filter(is_block).collect()
    } else if matches!(
        kind,
        "blockquote" | "callout" | "tableCell" | "tableHeader" | "listItem"
    ) {
        let mut blocks = group_blocks(children);
        if blocks.is_empty()
            || kind == "listItem"
                && blocks
                    .first()
                    .is_none_or(|node| node.node_type != "paragraph")
        {
            blocks.insert(0, Node::block("paragraph", Map::new(), vec![]));
        }
        blocks
    } else {
        children
    };
    Ok(vec![Node::block(kind, attrs, content)])
}
fn put_mark(marks: &mut Vec<Mark>, mark: Mark) {
    if !marks.iter().any(|old| old.mark_type == mark.mark_type) {
        marks.push(mark);
    }
}
fn is_block(node: &Node) -> bool {
    !matches!(node.node_type.as_str(), "text" | "hardBreak" | "inlineMath")
}
fn group_blocks(nodes: Vec<Node>) -> Vec<Node> {
    let mut result = Vec::new();
    let mut inline = Vec::new();
    for node in nodes {
        if is_block(&node) {
            if !inline.is_empty() {
                if inline.iter().any(|node: &Node| {
                    node.node_type != "text"
                        || node
                            .text
                            .as_deref()
                            .is_some_and(|text| !text.trim().is_empty())
                }) {
                    result.push(Node::block(
                        "paragraph",
                        Map::new(),
                        std::mem::take(&mut inline),
                    ));
                } else {
                    inline.clear();
                }
            }
            result.push(node);
        } else {
            inline.push(node);
        }
    }
    if !inline.is_empty()
        && inline.iter().any(|node| {
            node.node_type != "text"
                || node
                    .text
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
        })
    {
        result.push(Node::block("paragraph", Map::new(), inline));
    }
    result
}
fn text_content(node: NodeRef<'_, HtmlNode>) -> String {
    node.descendants()
        .filter_map(|node| node.value().as_text().map(|text| text.text.to_string()))
        .collect()
}
fn parse_image(node: NodeRef<'_, HtmlNode>, mut attrs: Map<String, Value>) -> Option<Node> {
    let el = node.value().as_element()?;
    let src = el.attr("src").filter(|src| safe_image_source(src))?;
    attrs.insert("src".into(), Value::String(src.into()));
    if let Some(alt) = el.attr("alt").filter(|alt| alt.len() <= 1000) {
        attrs.insert("alt".into(), Value::String(alt.into()));
    }
    for name in ["width", "height"] {
        if let Some(value) = el
            .attr(name)
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (1..=20000).contains(value))
        {
            attrs.insert(name.into(), Value::from(value));
        }
    }
    Some(Node::block("imageBlock", attrs, vec![]))
}
fn safe_provenance(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2048
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b",_-".contains(&ch))
}
fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"_-".contains(&ch))
}
fn uuid(value: &str) -> bool {
    value.len() == 36 && value.bytes().all(|ch| ch.is_ascii_hexdigit() || ch == b'-')
}
fn safe_html_href(value: &str) -> bool {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return true;
    }
    let route = value.split('#').next().unwrap_or(value);
    let pieces = route.split('/').collect::<Vec<_>>();
    pieces.len() == 5
        && pieces[0].is_empty()
        && pieces[1] == "projects"
        && uuid(pieces[2])
        && pieces[3] == "studio"
        && uuid(pieces[4])
        && !value.chars().any(char::is_whitespace)
}
fn safe_image_source(value: &str) -> bool {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return value.len() <= 2008
            && !value[8..].is_empty()
            && !value.chars().any(char::is_whitespace);
    }
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 5
        && parts[0].is_empty()
        && parts[1] == "api"
        && parts[2] == "media"
        && uuid(parts[3])
        && parts[4] == "content"
}
