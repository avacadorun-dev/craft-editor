//! Exact enabled Tiptap clipboardTextSerializer semantics, independent of Markdown export.
use super::{ClipboardError, RichSlice};
use crate::{DocPoint, Error, Result, Selection};
use craft_editor_model::{Document, Node, NodePath};

struct Entry<'a> {
    node: &'a Node,
    path: NodePath,
    start: usize,
    end: usize,
}
struct Index<'a> {
    entries: Vec<Entry<'a>>,
    size: usize,
}
impl<'a> Index<'a> {
    fn new(nodes: &'a [Node]) -> Result<Self> {
        let mut index = Self {
            entries: vec![],
            size: 0,
        };
        for (ordinal, node) in nodes.iter().enumerate() {
            index.size = index.visit(node, NodePath(vec![ordinal]), index.size)?;
        }
        Ok(index)
    }
    fn visit(&mut self, node: &'a Node, path: NodePath, start: usize) -> Result<usize> {
        let ordinal = self.entries.len();
        self.entries.push(Entry {
            node,
            path: path.clone(),
            start,
            end: start,
        });
        let end = if node.node_type == "text" {
            start.checked_add(
                node.text
                    .as_deref()
                    .unwrap_or_default()
                    .encode_utf16()
                    .count(),
            )
        } else if matches!(
            node.node_type.as_str(),
            "hardBreak"
                | "inlineMath"
                | "imageBlock"
                | "mathBlock"
                | "mermaidBlock"
                | "horizontalRule"
                | "sceneBreak"
        ) {
            start.checked_add(1)
        } else {
            let mut content = start.checked_add(1).ok_or(ClipboardError::ResourceLimit)?;
            for (ordinal, child) in node.children().iter().enumerate() {
                content = self.visit(child, path.child(ordinal), content)?;
            }
            content.checked_add(1)
        }
        .ok_or(ClipboardError::ResourceLimit)?;
        self.entries[ordinal].end = end;
        Ok(end)
    }
    fn entry(&self, path: &NodePath) -> Result<&Entry<'a>> {
        self.entries
            .iter()
            .find(|entry| entry.path == *path)
            .ok_or(Error::InvalidSelection)
    }
    fn point(&self, point: &DocPoint) -> Result<usize> {
        let entry = self
            .entries
            .iter()
            .find(|entry| {
                entry.node.block_id() == Some(point.block_id.as_str())
                    && matches!(
                        entry.node.node_type.as_str(),
                        "paragraph" | "heading" | "codeBlock"
                    )
            })
            .ok_or(Error::InvalidSelection)?;
        let mut remaining = point.offset.0;
        let mut position = entry
            .start
            .checked_add(1)
            .ok_or(ClipboardError::ResourceLimit)?;
        for child in entry.node.children() {
            if remaining == 0 {
                break;
            }
            let units = if child.node_type == "text" {
                let value = child.text.as_deref().unwrap_or_default();
                let length = value.chars().count();
                let take = remaining.min(length);
                remaining -= take;
                value.chars().take(take).map(char::len_utf16).sum::<usize>()
            } else {
                remaining -= 1;
                1
            };
            position = position
                .checked_add(units)
                .ok_or(ClipboardError::ResourceLimit)?;
        }
        if remaining != 0 {
            return Err(Error::InvalidSelection);
        }
        Ok(position)
    }
    fn text(&self, from: usize, to: usize) -> Result<String> {
        if from > to || to > self.size {
            return Err(Error::InvalidSelection);
        }
        let mut output = String::new();
        for entry in &self.entries {
            if entry.start >= to || entry.end <= from {
                continue;
            }
            let kind = entry.node.node_type.as_str();
            if !matches!(kind, "text" | "hardBreak" | "inlineMath") && entry.start > from {
                output.push_str("\n\n");
            }
            if kind == "hardBreak" {
                output.push('\n');
            } else if kind == "text" {
                let start = from.saturating_sub(entry.start);
                let end = to.saturating_sub(entry.start);
                let mut units = 0;
                for ch in entry.node.text.as_deref().unwrap_or_default().chars() {
                    let next = units + ch.len_utf16();
                    if units >= start && next <= end {
                        output.push(ch);
                    } else if (units < start && start < next) || (units < end && end < next) {
                        return Err(Error::InvalidSelection);
                    }
                    units = next;
                }
            }
        }
        Ok(output)
    }
}

pub(super) fn slice(slice: &RichSlice) -> Result<String> {
    let index = Index::new(&slice.content)?;
    let end = index
        .size
        .checked_sub(slice.open_end)
        .ok_or(ClipboardError::InvalidSlice)?;
    index.text(slice.open_start, end)
}
pub(super) fn selection(document: &Document, selection: &Selection) -> Result<String> {
    let index = Index::new(document.blocks())?;
    let anchor = index.point(&selection.anchor)?;
    let head = index.point(&selection.head)?;
    index.text(anchor.min(head), anchor.max(head))
}
pub(super) fn cells(document: &Document, paths: &[NodePath]) -> Result<String> {
    let index = Index::new(document.blocks())?;
    let mut ranges = paths
        .iter()
        .map(|path| {
            index
                .entry(path)
                .map(|entry| (entry.start + 1, entry.end - 1))
        })
        .collect::<Result<Vec<_>>>()?;
    ranges.sort_unstable();
    ranges
        .into_iter()
        .map(|(from, to)| index.text(from, to))
        .collect::<Result<Vec<_>>>()
        .map(|ranges| ranges.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    #[test]
    fn all_enabled_source_nodes_and_range_directions_match_the_installed_plain_serializer()
    -> TestResult {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/source-plain-copy.json"))?;
        assert_eq!(corpus["serializers"], serde_json::json!(["hardBreak"]));
        assert_eq!(corpus["nodes"].as_array().ok_or("missing nodes")?.len(), 21);
        for case in corpus["cases"].as_array().ok_or("missing cases")? {
            let document = Document::from_json(case["input"].clone())?;
            let index = Index::new(document.blocks())?;
            let actual = case["ranges"]
                .as_array()
                .ok_or("missing ranges")?
                .iter()
                .map(|range| {
                    index.text(
                        range["from"].as_u64().ok_or(Error::InvalidSelection)? as usize,
                        range["to"].as_u64().ok_or(Error::InvalidSelection)? as usize,
                    )
                })
                .collect::<Result<Vec<_>>>()?
                .join("\n\n");
            assert_eq!(
                actual,
                case["plain"].as_str().ok_or("missing plain")?,
                "{}",
                case["name"]
            );
        }
        Ok(())
    }
}
