use std::collections::HashSet;

use prosemirror::{dynamic::DynamicSchema, model::Node as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{Error, Result};

/// A canonical mark. Unknown attributes and fields survive round trips.
#[derive(Debug, Clone)]
pub struct Mark {
    /// Canonical mark name.
    pub mark_type: String,
    /// Attributes, retaining the difference between omitted and empty.
    pub attrs: Option<Map<String, Value>>,
    /// Opaque metadata outside the known fields.
    pub extra: Map<String, Value>,
    null_fields: HashSet<String>,
}

impl PartialEq for Mark {
    fn eq(&self, other: &Self) -> bool {
        self.mark_type == other.mark_type
            && self.attrs == other.attrs
            && self.extra == other.extra
            && null_presence(&self.attrs, &self.null_fields, "attrs")
                == null_presence(&other.attrs, &other.null_fields, "attrs")
    }
}

impl Mark {
    /// Construct an attribute-free mark.
    pub fn new(mark_type: impl Into<String>) -> Self {
        Self {
            mark_type: mark_type.into(),
            attrs: None,
            extra: Map::new(),
            null_fields: HashSet::new(),
        }
    }
    /// Merge supplied same-type attributes and opaque fields, retaining all unmentioned data.
    ///
    /// The complete resulting mark is validated. A type mismatch or invalid href/metadata fails
    /// without changing either input. Explicit supplied JSON null remains an explicit value.
    pub fn with_patched_attributes(&self, patch: &Self) -> Result<Self> {
        if self.mark_type != patch.mark_type {
            return Err(Error::InvalidDocument("mark_type_mismatch".into()));
        }
        let mut updated = self.clone();
        if let Some(attrs) = &patch.attrs {
            updated.attrs.get_or_insert_default().extend(attrs.clone());
        }
        updated.extra.extend(patch.extra.clone());
        validate_mark(&updated)?;
        Ok(updated)
    }
}

/// A lossless ProseMirror-shaped node, independent of rendering and storage.
#[derive(Debug, Clone)]
pub struct Node {
    /// Canonical node name.
    pub node_type: String,
    /// Attributes, including application and extension metadata.
    pub attrs: Option<Map<String, Value>>,
    /// Explicit marks, retaining omitted versus empty arrays.
    pub marks: Option<Vec<Mark>>,
    /// Text-node content, without Unicode normalization.
    pub text: Option<String>,
    /// Ordered children, retaining omitted versus empty arrays.
    pub content: Option<Vec<Node>>,
    /// Opaque node fields not understood by this model.
    pub extra: Map<String, Value>,
    null_fields: HashSet<String>,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.node_type == other.node_type
            && self.attrs == other.attrs
            && self.marks == other.marks
            && self.text == other.text
            && self.content == other.content
            && self.extra == other.extra
            && self.same_null_presence(other)
    }
}

// Public Option fields may replace a decoded null with a concrete value. The
// remembered null only affects serialization while that Option remains None.
fn null_presence<T>(value: &Option<T>, null_fields: &HashSet<String>, field: &str) -> bool {
    value.is_none() && null_fields.contains(field)
}

impl Node {
    /// Decode bounded canonical nodes for a potentially open clipboard fragment.
    ///
    /// Attributes, marks, identities, wire shape and model resource bounds are checked. Open
    /// edge wrappers may be incomplete; their child grammar and table grid must be validated
    /// against the complete candidate document before any insertion transaction can commit.
    pub fn decode_open_fragment(value: Value) -> Result<Vec<Self>> {
        let values = value
            .as_array()
            .ok_or_else(|| Error::InvalidDocument("fragment_array".into()))?;
        for value in values {
            validate_wire(value, 0)?;
        }
        let nodes: Vec<Self> = serde_json::from_value(value)?;
        let mut ids = HashSet::new();
        let mut budget = 100_000;
        for node in &nodes {
            if node.node_type == "doc" {
                return invalid("fragment_root");
            }
            validate_node(node, 4, 0, &mut ids, &mut budget, true)?;
        }
        Ok(nodes)
    }

    /// Minimum canonical schema version needed by this complete subtree.
    /// This does not change or validate the node; transaction promotion still
    /// validates the complete resulting document before it can be committed.
    pub fn required_schema_version(&self) -> Result<u64> {
        crate::structural::check_subtree_bounds(self)?;
        Ok(required_version(self))
    }

    pub(crate) fn same_null_presence(&self, other: &Self) -> bool {
        null_presence(&self.attrs, &self.null_fields, "attrs")
            == null_presence(&other.attrs, &other.null_fields, "attrs")
            && null_presence(&self.marks, &self.null_fields, "marks")
                == null_presence(&other.marks, &other.null_fields, "marks")
            && null_presence(&self.text, &self.null_fields, "text")
                == null_presence(&other.text, &other.null_fields, "text")
            && null_presence(&self.content, &self.null_fields, "content")
                == null_presence(&other.content, &other.null_fields, "content")
    }

    /// Compare metadata that a content merge would otherwise discard.
    pub(crate) fn compatible_envelope(&self, other: &Self, ignored_attrs: &[&str]) -> bool {
        let stripped_attrs = |node: &Self| {
            let mut attrs = node.attrs.clone();
            if let Some(attrs) = &mut attrs {
                attrs.retain(|key, _| !ignored_attrs.contains(&key.as_str()));
            }
            attrs
        };
        stripped_attrs(self) == stripped_attrs(other)
            && self.marks == other.marks
            && self.text == other.text
            && self.extra == other.extra
            && null_presence(&self.attrs, &self.null_fields, "attrs")
                == null_presence(&other.attrs, &other.null_fields, "attrs")
            && null_presence(&self.marks, &self.null_fields, "marks")
                == null_presence(&other.marks, &other.null_fields, "marks")
            && null_presence(&self.text, &self.null_fields, "text")
                == null_presence(&other.text, &other.null_fields, "text")
    }

    /// Inherit unspecified leaf fields, retaining explicit template envelopes.
    pub(crate) fn merge_template_leaf_fields(&mut self, template: &Self) -> Result<()> {
        fn merge<T: Clone + PartialEq>(
            destination: &mut Option<T>,
            destination_nulls: &mut HashSet<String>,
            source: &Option<T>,
            source_nulls: &HashSet<String>,
            field: &str,
        ) -> Result<()> {
            let source_null = null_presence(source, source_nulls, field);
            if source.is_none() && !source_null {
                return Ok(());
            }
            let destination_null = null_presence(destination, destination_nulls, field);
            if (destination.is_some() || destination_null)
                && (destination.as_ref() != source.as_ref() || destination_null != source_null)
            {
                return Err(Error::UnsupportedEdit);
            }
            *destination = source.clone();
            if source_null {
                destination_nulls.insert(field.to_owned());
            } else {
                destination_nulls.remove(field);
            }
            Ok(())
        }
        merge(
            &mut self.marks,
            &mut self.null_fields,
            &template.marks,
            &template.null_fields,
            "marks",
        )?;
        merge(
            &mut self.text,
            &mut self.null_fields,
            &template.text,
            &template.null_fields,
            "text",
        )
    }

    /// Construct a literal text leaf. Validation rejects empty leaves.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            node_type: "text".into(),
            attrs: None,
            marks: None,
            text: Some(text.into()),
            content: None,
            extra: Map::new(),
            null_fields: HashSet::new(),
        }
    }
    /// Construct a container or atom from canonical attributes and children.
    pub fn block(
        node_type: impl Into<String>,
        attrs: Map<String, Value>,
        content: Vec<Node>,
    ) -> Self {
        Self {
            node_type: node_type.into(),
            attrs: Some(attrs),
            marks: None,
            text: None,
            content: Some(content),
            extra: Map::new(),
            null_fields: HashSet::new(),
        }
    }
    /// Construct a paragraph with stable identity and an empty provenance list.
    pub fn paragraph(id: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let children = if text.is_empty() {
            vec![]
        } else {
            vec![Self::text(text)]
        };
        Self::block(
            "paragraph",
            Map::from_iter([
                ("blockId".into(), Value::String(id.into())),
                ("sourceRefs".into(), json!([])),
            ]),
            children,
        )
    }
    /// Borrow ordered children, treating an omitted field as empty.
    pub fn children(&self) -> &[Node] {
        self.content.as_deref().unwrap_or_default()
    }
    /// Resolve a string attribute without coercing arbitrary JSON.
    pub fn string_attr(&self, name: &str) -> Option<&str> {
        self.attrs.as_ref()?.get(name)?.as_str()
    }
    /// Stable identity when explicitly present.
    pub fn block_id(&self) -> Option<&str> {
        self.string_attr("blockId")
    }
    pub(crate) fn can_coalesce_text(&self, other: &Self) -> bool {
        self.node_type == "text"
            && other.node_type == "text"
            && self.text.as_ref().is_some_and(|text| !text.is_empty())
            && other.text.as_ref().is_some_and(|text| !text.is_empty())
            && self.attrs == other.attrs
            && self.marks == other.marks
            && self.content == other.content
            && self.extra == other.extra
            && self.same_null_presence(other)
    }
    /// The shared inline projection: hard breaks are newlines; inline atoms are U+FFFC.
    pub fn inline_text(&self) -> String {
        match self.node_type.as_str() {
            "text" => self.text.clone().unwrap_or_default(),
            "hardBreak" => "\n".into(),
            "inlineMath" => "\u{fffc}".into(),
            _ => self.children().iter().map(Self::inline_text).collect(),
        }
    }
}

/// A validated rich tree retaining original unknown JSON metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub(crate) root: Node,
}

impl Document {
    /// Decode and validate a canonical JSON value without rewriting its fields.
    pub fn from_json(value: Value) -> Result<Self> {
        validate_wire(&value, 0)?;
        Self::from_root(serde_json::from_value(value)?)
    }
    /// Validate an already constructed root node.
    pub fn from_root(root: Node) -> Result<Self> {
        let document = Self { root };
        document.validate()?;
        Ok(document)
    }
    /// Construct a schema-v1 document with one identified paragraph.
    pub fn plain(id: impl Into<String>, text: impl Into<String>) -> Result<Self> {
        Self::from_root(Node::block(
            "doc",
            Map::from_iter([("schemaVersion".into(), json!(1))]),
            vec![Node::paragraph(id, text)],
        ))
    }
    /// Serialize the authoritative tree, including unknown fields and attributes.
    pub fn to_json(&self) -> Result<Value> {
        Ok(serde_json::to_value(&self.root)?)
    }
    /// Borrow the canonical root.
    pub fn root(&self) -> &Node {
        &self.root
    }
    /// Borrow the ordered top-level blocks.
    pub fn blocks(&self) -> &[Node] {
        self.root.children()
    }
    /// Resolve a unique identified node at any depth.
    pub fn block(&self, block_id: &str) -> Result<&Node> {
        find_node(&self.root, block_id).ok_or(Error::BlockNotFound)
    }
    /// Project one identified block to the common scalar-addressable inline text.
    pub fn block_text(&self, block_id: &str) -> Result<String> {
        Ok(self.block(block_id)?.inline_text())
    }
    /// A derived display/search projection, never a substitute for canonical JSON.
    pub fn plain_text(&self) -> String {
        self.blocks()
            .iter()
            .map(Node::inline_text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    pub(crate) fn block_mut(&mut self, block_id: &str) -> Result<&mut Node> {
        find_node_mut(&mut self.root, block_id).ok_or(Error::BlockNotFound)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        crate::structural::check_subtree_bounds(&self.root)?;
        if self.root.node_type != "doc" {
            return invalid("root_type");
        }
        let version = schema_version(&self.root)?;
        let mut ids = HashSet::new();
        let mut budget = 100_000;
        validate_node(&self.root, version, 0, &mut ids, &mut budget, false)?;
        let schema = DynamicSchema::from_json(&schema_spec(&self.root))
            .map_err(|_| Error::InvalidDocument("schema_spec".into()))?;
        let mut json = self.to_json()?;
        project_for_validation(&mut json);
        schema.with_types(|| {
            let parsed = schema
                .node_from_json(&json)
                .map_err(|_| Error::InvalidDocument("structure".into()))?;
            parsed
                .check()
                .map_err(|_| Error::InvalidDocument("structure".into()))
        })
    }
    pub(crate) fn promote_schema(&mut self) -> Result<()> {
        crate::structural::check_subtree_bounds(&self.root)?;
        let required = required_version(&self.root);
        if schema_version(&self.root)? < required {
            self.root
                .attrs
                .get_or_insert_default()
                .insert("schemaVersion".into(), json!(required));
        }
        Ok(())
    }
}

fn invalid<T>(code: &str) -> Result<T> {
    Err(Error::InvalidDocument(code.into()))
}
fn schema_version(root: &Node) -> Result<u64> {
    match root
        .attrs
        .as_ref()
        .and_then(|attrs| attrs.get("schemaVersion"))
    {
        None => Ok(1),
        Some(value) => integer(value)
            .filter(|version| (1..=4).contains(version))
            .ok_or_else(|| Error::InvalidDocument("schema_version".into())),
    }
}
fn validate_wire(value: &Value, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(Error::ResourceLimit);
    }
    let object = value
        .as_object()
        .ok_or_else(|| Error::InvalidDocument("node_object".into()))?;
    for name in ["attrs", "content", "marks"] {
        if let Some(field) = object.get(name)
            && !field.is_null()
            && ((name == "attrs" && !field.is_object()) || (name != "attrs" && !field.is_array()))
        {
            return invalid("field_type");
        }
    }
    if let Some(children) = object.get("content").and_then(Value::as_array) {
        for child in children {
            validate_wire(child, depth + 1)?;
        }
    }
    Ok(())
}
fn validate_node(
    node: &Node,
    version: u64,
    depth: usize,
    ids: &mut HashSet<String>,
    budget: &mut usize,
    open_fragment: bool,
) -> Result<()> {
    if depth > 64 || *budget == 0 {
        return Err(Error::ResourceLimit);
    }
    *budget -= 1;
    if !NODES.contains(&node.node_type.as_str()) {
        return invalid("node_type");
    }
    if node
        .extra
        .keys()
        .any(|key| ["type", "attrs", "content", "marks", "text"].contains(&key.as_str()))
    {
        return invalid("reserved_extra_field");
    }
    if let Some(id) = node.attrs.as_ref().and_then(|attrs| attrs.get("blockId")) {
        let id = id
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 160)
            .ok_or_else(|| Error::InvalidDocument("block_id".into()))?;
        if !ids.insert(id.to_owned()) {
            return invalid("duplicate_block_id");
        }
    }
    if let Some(refs) = node
        .attrs
        .as_ref()
        .and_then(|attrs| attrs.get("sourceRefs"))
        && !refs.is_null()
        && !refs
            .as_array()
            .is_some_and(|refs| refs.iter().all(Value::is_string))
    {
        return invalid("source_refs");
    }
    if node.node_type == "text" {
        if node.text.as_ref().is_none_or(String::is_empty) || !node.children().is_empty() {
            return invalid("text_leaf");
        }
    } else if node.text.is_some() {
        return invalid("nontext_text");
    }
    if let Some(text) = &node.text
        && text.len() > 8 * 1024 * 1024
    {
        return Err(Error::ResourceLimit);
    }
    if node.node_type == "heading"
        && !node
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("level"))
            .and_then(integer)
            .is_some_and(|level| (1..=6).contains(&level))
    {
        return invalid("heading_level");
    }
    if node.node_type == "callout"
        && !node
            .string_attr("kind")
            .is_some_and(|kind| ["info", "idea", "warning", "todo"].contains(&kind))
    {
        return invalid("callout_kind");
    }
    if node.node_type == "codeBlock"
        && let Some(value) = node.attrs.as_ref().and_then(|attrs| attrs.get("language"))
        && !value.is_null()
        && !value.as_str().is_some_and(|language| {
            language.len() <= 40
                && language
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || b"_+#.-".contains(&ch))
        })
    {
        return invalid("code_language");
    }
    if ["tableCell", "tableHeader"].contains(&node.node_type.as_str()) {
        for attr in ["colspan", "rowspan"] {
            if let Some(value) = node.attrs.as_ref().and_then(|attrs| attrs.get(attr))
                && !integer(value).is_some_and(|number| (1..=100).contains(&number))
            {
                return invalid("table_span");
            }
        }
        if let Some(value) = node.attrs.as_ref().and_then(|attrs| attrs.get("align"))
            && !value.is_null()
            && !value
                .as_str()
                .is_some_and(|align| ["left", "right", "center"].contains(&align))
        {
            return invalid("table_alignment");
        }
        if let Some(value) = node.attrs.as_ref().and_then(|attrs| attrs.get("colwidth"))
            && !value.is_null()
            && !value.as_array().is_some_and(|widths| {
                widths
                    .iter()
                    .all(|width| integer(width).is_some_and(|width| width <= 20_000))
            })
        {
            return invalid("table_width");
        }
    }
    if node.node_type == "table" && !open_fragment {
        crate::tables::TableGrid::new(node)?;
    }
    if node.node_type == "imageBlock" {
        validate_string(node, "src", 2_000, true)?;
        for attr in ["alt", "caption", "assetId"] {
            validate_string(node, attr, 1_000, false)?;
        }
        for attr in ["width", "height"] {
            if let Some(value) = node.attrs.as_ref().and_then(|attrs| attrs.get(attr))
                && !value.is_null()
                && !integer(value).is_some_and(|dimension| (1..=20_000).contains(&dimension))
            {
                return invalid("image_dimension");
            }
        }
    }
    if node.node_type == "mermaidBlock" {
        validate_string(node, "code", 100_000, true)?;
        validate_string(node, "caption", 1_000, false)?;
    }
    if ["inlineMath", "mathBlock"].contains(&node.node_type.as_str()) {
        if !node
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("latex"))
            .is_some_and(Value::is_string)
        {
            return invalid("formula_source");
        }
        validate_string(node, "latex", 10_000, false)?;
        validate_string(node, "description", 1_000, false)?;
    }
    if let Some(marks) = &node.marks {
        let mut kinds = HashSet::new();
        for mark in marks {
            validate_mark(mark)?;
            if !kinds.insert(&mark.mark_type) {
                return invalid("duplicate_mark");
            }
        }
    }
    if required_version(node) > version {
        return invalid("schema_version_too_low");
    }
    for child in node.children() {
        validate_node(child, version, depth + 1, ids, budget, open_fragment)?;
    }
    Ok(())
}
/// Match the original Tiptap Link default URI policy without opening or fetching the target.
///
/// The original application does not extend its default protocols. HTML clipboard import has
/// an intentionally stricter source sanitizer policy and must not substitute this broader check.
pub fn is_allowed_link_uri(uri: &str) -> bool {
    let normalized = uri
        .chars()
        .filter(|ch| {
            !matches!(
                *ch, '\u{0000}'..='\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{180e}'
                    | '\u{2000}'..='\u{2029}' | '\u{205f}' | '\u{3000}'
            )
        })
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }
    const PROTOCOLS: &[&str] = &[
        "http", "https", "ftp", "ftps", "mailto", "tel", "callto", "sms", "cid", "xmpp",
    ];
    if PROTOCOLS.iter().any(|protocol| {
        normalized
            .strip_prefix(protocol)
            .is_some_and(|tail| tail.starts_with(':'))
    }) {
        return true;
    }
    if normalized
        .chars()
        .next()
        .is_some_and(|ch| !ch.is_ascii_alphabetic())
    {
        return true;
    }
    for ch in normalized.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-') {
            continue;
        }
        return ch != ':';
    }
    true
}

pub(crate) fn validate_mark(mark: &Mark) -> Result<()> {
    if !MARKS.contains(&mark.mark_type.as_str()) {
        return invalid("mark_type");
    }
    if mark
        .extra
        .keys()
        .any(|key| ["type", "attrs"].contains(&key.as_str()))
    {
        return invalid("reserved_mark_field");
    }
    if mark.mark_type == "link" {
        let href = mark
            .attrs
            .as_ref()
            .and_then(|attrs| attrs.get("href"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidDocument("link_href".into()))?;
        if !is_allowed_link_uri(href) {
            return invalid("link_scheme");
        }
    }
    Ok(())
}
fn validate_string(node: &Node, attr: &str, maximum: usize, required: bool) -> Result<()> {
    match node.attrs.as_ref().and_then(|attrs| attrs.get(attr)) {
        None | Some(Value::Null) if !required => Ok(()),
        Some(Value::String(value))
            if value.len() <= maximum && (!required || !value.is_empty()) =>
        {
            Ok(())
        }
        _ => invalid("string_attribute"),
    }
}
fn required_version(node: &Node) -> u64 {
    let own = match node.node_type.as_str() {
        "inlineMath" | "mathBlock" => 4,
        "heading"
            if node
                .attrs
                .as_ref()
                .and_then(|attrs| attrs.get("level"))
                .and_then(integer)
                .is_some_and(|level| level >= 4) =>
        {
            3
        }
        "codeBlock" | "table" | "tableRow" | "tableHeader" | "tableCell" | "imageBlock"
        | "mermaidBlock" | "callout" => 2,
        _ if node
            .marks
            .as_ref()
            .is_some_and(|marks| marks.iter().any(|mark| mark.mark_type == "code")) =>
        {
            2
        }
        _ => 1,
    };
    node.children()
        .iter()
        .map(required_version)
        .fold(own, u64::max)
}
pub(crate) fn find_node<'a>(node: &'a Node, id: &str) -> Option<&'a Node> {
    if node.block_id() == Some(id) {
        return Some(node);
    }
    node.children().iter().find_map(|node| find_node(node, id))
}
fn find_node_mut<'a>(node: &'a mut Node, id: &str) -> Option<&'a mut Node> {
    if node.block_id() == Some(id) {
        return Some(node);
    }
    node.content
        .as_mut()?
        .iter_mut()
        .find_map(|node| find_node_mut(node, id))
}
const NODES: &[&str] = &[
    "doc",
    "paragraph",
    "heading",
    "blockquote",
    "bulletList",
    "orderedList",
    "listItem",
    "hardBreak",
    "horizontalRule",
    "sceneBreak",
    "text",
    "codeBlock",
    "table",
    "tableRow",
    "tableHeader",
    "tableCell",
    "imageBlock",
    "mermaidBlock",
    "callout",
    "inlineMath",
    "mathBlock",
];
const MARKS: &[&str] = &["bold", "italic", "underline", "strike", "link", "code"];
fn schema_spec(root: &Node) -> Value {
    let mut spec = json!({"nodes":{
        "doc":{"content":"block+"},
        "paragraph":{"content":"inline*","group":"block"},
        "heading":{"content":"inline*","group":"block"},
        "blockquote":{"content":"block+","group":"block"},
        "bulletList":{"content":"listItem+","group":"block"},
        "orderedList":{"content":"listItem+","group":"block"},
        "listItem":{"content":"paragraph block*"},
        "hardBreak":{"inline":true,"group":"inline"},
        "horizontalRule":{"group":"block","atom":true},
        "sceneBreak":{"group":"block","atom":true},
        "text":{"group":"inline"},
        "codeBlock":{"content":"text*","group":"block","marks":""},
        "table":{"content":"tableRow+","group":"block"},
        "tableRow":{"content":"(tableCell | tableHeader)*"},
        "tableHeader":{"content":"block+"},"tableCell":{"content":"block+"},
        "imageBlock":{"group":"block","atom":true},
        "mermaidBlock":{"group":"block","atom":true},
        "callout":{"content":"block+","group":"block"},
        "inlineMath":{"inline":true,"group":"inline","atom":true},
        "mathBlock":{"group":"block","atom":true}
    },"marks":{"bold":{},"italic":{},"underline":{},"strike":{},"link":{},"code":{}}});
    declare_attrs(root, &mut spec);
    spec
}

fn declare_attrs(node: &Node, spec: &mut Value) {
    if let Some(attrs) = &node.attrs {
        for key in attrs.keys() {
            spec["nodes"][&node.node_type]["attrs"][key] = json!({"default":null});
        }
    }
    if let Some(marks) = &node.marks {
        for mark in marks {
            if let Some(attrs) = &mark.attrs {
                for key in attrs.keys() {
                    spec["marks"][&mark.mark_type]["attrs"][key] = json!({"default":null});
                }
            }
        }
    }
    for child in node.children() {
        declare_attrs(child, spec);
    }
}

fn take_optional<T: DeserializeOwned>(
    object: &mut Map<String, Value>,
    name: &str,
    null_fields: &mut HashSet<String>,
) -> std::result::Result<Option<T>, &'static str> {
    match object.remove(name) {
        None => Ok(None),
        Some(Value::Null) => {
            null_fields.insert(name.to_owned());
            Ok(None)
        }
        Some(value) => serde_json::from_value(value)
            .map(Some)
            .map_err(|_| "invalid_envelope_field"),
    }
}
fn put_optional<T: Serialize>(
    object: &mut Map<String, Value>,
    name: &str,
    field: &Option<T>,
    null_fields: &HashSet<String>,
) -> std::result::Result<(), serde_json::Error> {
    if let Some(value) = field {
        object.insert(name.to_owned(), serde_json::to_value(value)?);
    } else if null_fields.contains(name) {
        object.insert(name.to_owned(), Value::Null);
    }
    Ok(())
}
impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        if self
            .extra
            .keys()
            .any(|key| ["type", "attrs", "text", "content", "marks"].contains(&key.as_str()))
        {
            return Err(serde::ser::Error::custom("reserved_node_extra_field"));
        }
        let mut object = self.extra.clone();
        object.insert("type".into(), Value::String(self.node_type.clone()));
        put_optional(&mut object, "attrs", &self.attrs, &self.null_fields)
            .map_err(serde::ser::Error::custom)?;
        put_optional(&mut object, "marks", &self.marks, &self.null_fields)
            .map_err(serde::ser::Error::custom)?;
        put_optional(&mut object, "text", &self.text, &self.null_fields)
            .map_err(serde::ser::Error::custom)?;
        put_optional(&mut object, "content", &self.content, &self.null_fields)
            .map_err(serde::ser::Error::custom)?;
        object.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let mut object = Map::<String, Value>::deserialize(deserializer)?;
        let node_type = object
            .remove("type")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| serde::de::Error::custom("missing_node_type"))?;
        let mut null_fields = HashSet::new();
        let attrs = take_optional(&mut object, "attrs", &mut null_fields)
            .map_err(serde::de::Error::custom)?;
        let marks = take_optional(&mut object, "marks", &mut null_fields)
            .map_err(serde::de::Error::custom)?;
        let text = take_optional(&mut object, "text", &mut null_fields)
            .map_err(serde::de::Error::custom)?;
        let content = take_optional(&mut object, "content", &mut null_fields)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            node_type,
            attrs,
            marks,
            text,
            content,
            extra: object,
            null_fields,
        })
    }
}
impl Serialize for Mark {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        if self
            .extra
            .keys()
            .any(|key| ["type", "attrs"].contains(&key.as_str()))
        {
            return Err(serde::ser::Error::custom("reserved_mark_extra_field"));
        }
        let mut object = self.extra.clone();
        object.insert("type".into(), Value::String(self.mark_type.clone()));
        put_optional(&mut object, "attrs", &self.attrs, &self.null_fields)
            .map_err(serde::ser::Error::custom)?;
        object.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Mark {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let mut object = Map::<String, Value>::deserialize(deserializer)?;
        let mark_type = object
            .remove("type")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| serde::de::Error::custom("missing_mark_type"))?;
        let mut null_fields = HashSet::new();
        let attrs = take_optional(&mut object, "attrs", &mut null_fields)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            mark_type,
            attrs,
            extra: object,
            null_fields,
        })
    }
}

fn project_for_validation(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for name in ["attrs", "marks", "text", "content"] {
        if object.get(name).is_some_and(Value::is_null) {
            object.remove(name);
        }
    }
    if let Some(marks) = object.get_mut("marks").and_then(Value::as_array_mut) {
        for mark in marks {
            if let Some(object) = mark.as_object_mut()
                && object.get("attrs").is_some_and(Value::is_null)
            {
                object.remove("attrs");
            }
        }
    }
    if let Some(children) = object.get_mut("content").and_then(Value::as_array_mut) {
        for child in children {
            project_for_validation(child);
        }
    }
}

pub(crate) fn integer(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|number| {
                number.is_finite()
                    && *number >= 0.0
                    && number.fract() == 0.0
                    && *number < u64::MAX as f64
            })
            .map(|number| number as u64)
    })
}
