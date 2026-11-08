//! One table-local projection for native pixels, semantic hits and caret/IME geometry.
use super::*;
use std::hash::{Hash, Hasher};

/// Native component gutter reserved after an overflowing table, without authored spacer content.
pub const TABLE_SCROLL_GUTTER: f32 = 26.0;

/// One bounded horizontal table viewport in the shared document coordinate space.
#[derive(Debug, Clone)]
pub struct TableViewport {
    /// Exact table location in this immutable measured revision.
    pub table_path: NodePath,
    /// Session-local presentation identity, never persistence or edit authority.
    pub identity: String,
    /// Visible component viewport after ancestor translation/clipping.
    pub rect: GeometryRect,
    /// Scrollbar gutter after the table's actual cell boxes.
    pub scrollbar: GeometryRect,
    /// Full measured logical track width, without fit-shrinking.
    pub content_width: f32,
    /// Current presentation-only horizontal offset.
    pub offset: f32,
    /// Largest valid offset in the component's own coordinate frame.
    pub maximum: f32,
}
#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub path: NodePath,
    key: String,
    raw: GeometryRect,
    width: f32,
    offset: f32,
}
#[derive(Default)]
pub(super) struct Viewports {
    entries: Vec<Entry>,
    index: HashMap<NodePath, usize>,
}
fn intersection(a: GeometryRect, b: GeometryRect) -> GeometryRect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    GeometryRect {
        page: a.page,
        x,
        y,
        width: (a.x + a.width).min(b.x + b.width).max(x) - x,
        height: (a.y + a.height).min(b.y + b.height).max(y) - y,
    }
}
fn contains(rect: GeometryRect, x: f32, y: f32) -> bool {
    rect.width > 0.0
        && rect.height > 0.0
        && x >= rect.x
        && x <= rect.x + rect.width
        && y >= rect.y
        && y <= rect.y + rect.height
}
impl Viewports {
    pub fn new(
        source: &Document,
        cells: &[CellGeometry],
        available: &HashMap<NodePath, f32>,
    ) -> Result<Self> {
        let mut bounds: HashMap<NodePath, GeometryRect> = HashMap::new();
        for cell in cells {
            if cell.rect.page != 0 || cell.canonical_path.0.len() < 2 {
                continue;
            }
            let path = NodePath(cell.canonical_path.0[..cell.canonical_path.0.len() - 2].to_vec());
            bounds
                .entry(path)
                .and_modify(|rect| {
                    let right = (rect.x + rect.width).max(cell.rect.x + cell.rect.width);
                    let bottom = (rect.y + rect.height).max(cell.rect.y + cell.rect.height);
                    rect.x = rect.x.min(cell.rect.x);
                    rect.y = rect.y.min(cell.rect.y);
                    rect.width = right - rect.x;
                    rect.height = bottom - rect.y;
                })
                .or_insert(cell.rect);
        }
        let mut entries = Vec::new();
        for (path, raw) in bounds {
            let width = *available.get(&path).ok_or(Error::InvalidGrid)?;
            if raw.width <= width + 0.5 {
                continue;
            }
            let node = source.node(&path)?;
            let key = if let Some(id) = node.block_id() {
                format!("table:{id}")
            } else {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                serde_json::to_string(node)
                    .map_err(craft_editor_model::Error::from)?
                    .hash(&mut hash);
                format!("anonymous:{:?}:{:x}", path.0, hash.finish())
            };
            entries.push(Entry {
                path,
                key,
                raw,
                width,
                offset: 0.0,
            });
        }
        entries.sort_by(|a, b| a.path.0.cmp(&b.path.0));
        let index = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), index))
            .collect();
        Ok(Self { entries, index })
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    fn ancestors<'a>(&'a self, path: &'a NodePath) -> impl Iterator<Item = &'a Entry> {
        (0..path.0.len()).filter_map(|depth| {
            self.index
                .get(&NodePath(path.0[..depth].to_vec()))
                .map(|index| &self.entries[*index])
        })
    }
    pub fn shift(&self, path: &NodePath) -> f32 {
        self.ancestors(path).map(|entry| entry.offset).sum()
    }
    pub fn unbounded(&self, path: &NodePath, mut rect: GeometryRect) -> GeometryRect {
        rect.x -= self.shift(path);
        rect
    }
    pub fn clip(&self, path: &NodePath) -> Option<GeometryRect> {
        self.ancestors(path)
            .map(|entry| {
                let mut rect = entry.raw;
                rect.width = entry.width;
                self.unbounded(&entry.path, rect)
            })
            .reduce(intersection)
    }
    pub fn rect(&self, path: &NodePath, rect: GeometryRect) -> Option<GeometryRect> {
        let rect = self.unbounded(path, rect);
        let rect = self
            .clip(path)
            .map_or(rect, |clip| intersection(rect, clip));
        (rect.width > 0.0 && rect.height > 0.0).then_some(rect)
    }
    pub fn scope_at(&self, page: usize, x: f32, y: f32) -> Option<&Entry> {
        self.entries
            .iter()
            .filter(|entry| {
                let mut rect = entry.raw;
                rect.width = entry.width;
                self.rect(&entry.path, rect)
                    .is_some_and(|rect| rect.page == page && contains(rect, x, y))
            })
            .max_by_key(|entry| entry.path.0.len())
    }
    pub fn inverse(&self, page: usize, x: f32, y: f32) -> (f32, f32) {
        self.scope_at(page, x, y)
            .map_or((x, y), |entry| (x + self.shift(&entry.path.child(0)), y))
    }
    pub fn scope_for_rect(&self, rect: GeometryRect) -> Option<NodePath> {
        self.entries
            .iter()
            .filter(|entry| {
                rect.page == entry.raw.page
                    && rect.x >= entry.raw.x - 1.0
                    && rect.y >= entry.raw.y - 1.0
                    && rect.x + rect.width <= entry.raw.x + entry.raw.width + 1.0
                    && rect.y + rect.height <= entry.raw.y + entry.raw.height + 1.0
            })
            .max_by_key(|entry| entry.path.0.len())
            .map(|entry| entry.path.child(0))
    }
    pub fn list(&self) -> Vec<TableViewport> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let mut raw = entry.raw;
                raw.width = entry.width;
                let rect = self.rect(&entry.path, raw)?;
                let mut scrollbar = raw;
                scrollbar.y += raw.height + 3.0;
                scrollbar.height = 20.0;
                let scrollbar = self.rect(&entry.path, scrollbar)?;
                Some(TableViewport {
                    table_path: entry.path.clone(),
                    identity: entry.key.clone(),
                    rect,
                    scrollbar,
                    content_width: entry.raw.width,
                    offset: entry.offset,
                    maximum: (entry.raw.width - entry.width).max(0.0),
                })
            })
            .collect()
    }
    pub fn set(&mut self, path: &NodePath, offset: f32) -> Result<bool> {
        if !offset.is_finite() {
            return Err(Error::InvalidSize);
        }
        let index = *self.index.get(path).ok_or(Error::InvalidGrid)?;
        let entry = &mut self.entries[index];
        let offset = offset.clamp(0.0, (entry.raw.width - entry.width).max(0.0));
        let changed = (entry.offset - offset).abs() > 0.01;
        entry.offset = offset;
        Ok(changed)
    }
    pub fn offsets(&self) -> HashMap<String, f32> {
        self.entries
            .iter()
            .map(|entry| (entry.key.clone(), entry.offset))
            .collect()
    }
    pub fn restore(&mut self, offsets: &HashMap<String, f32>) {
        for entry in &mut self.entries {
            entry.offset = offsets
                .get(&entry.key)
                .copied()
                .filter(|offset| offset.is_finite())
                .unwrap_or(0.0)
                .clamp(0.0, (entry.raw.width - entry.width).max(0.0));
        }
    }
    pub fn reveal(&mut self, path: &NodePath, raw: GeometryRect) -> bool {
        let scopes: Vec<_> = self
            .ancestors(path)
            .map(|entry| entry.path.clone())
            .collect();
        let mut changed = false;
        for scope in scopes.into_iter().rev() {
            let index = self.index[&scope];
            let entry = &self.entries[index];
            let mut viewport = entry.raw;
            viewport.width = entry.width;
            let viewport = self.unbounded(&entry.path, viewport);
            let target = self.unbounded(path, raw);
            let delta = if target.x < viewport.x + 6.0 {
                target.x - viewport.x - 6.0
            } else if target.x + target.width > viewport.x + viewport.width - 6.0 {
                target.x + target.width - viewport.x - viewport.width + 6.0
            } else {
                0.0
            };
            let offset = entry.offset + delta;
            if let Ok(updated) = self.set(&scope, offset) {
                changed |= updated;
            }
        }
        changed
    }
}

#[cfg(feature = "wordcraft-canvas")]
#[derive(Default)]
pub(super) struct ViewPaint {
    pub pages: Vec<Vec<wordcraft_layout::display::Draw>>,
    scopes: Vec<(usize, usize, Vec<NodePath>)>,
}
#[cfg(feature = "wordcraft-canvas")]
impl ViewPaint {
    pub fn new(geometry: &FlowGeometry) -> Self {
        use wordcraft_layout::display::{DisplayOptions, Draw, ItemDisplay};
        let mut paint = Self::default();
        for (page_number, page) in geometry.measured.pages.iter().enumerate() {
            let display = ItemDisplay::new(&geometry.upstream, page);
            let mut draws = Vec::new();
            for item in &page.items {
                let mut items = display.draws(item, &DisplayOptions::default());
                if items.is_empty() {
                    continue;
                }
                let path = geometry.canonical_item_path(item).unwrap_or_default();
                let ancestors: Vec<_> = geometry.viewports.ancestors(&path).collect();
                for entry in ancestors.iter().rev() {
                    let mut rect = entry.raw;
                    rect.width = entry.width;
                    items = vec![Draw::Clip {
                        rect: wordcraft_geom::Rect::new(rect.x, rect.y, rect.width, rect.height),
                        items: vec![Draw::Turned {
                            x: -entry.offset,
                            y: 0.0,
                            turn: wordcraft_doc::props::TextDirection::Horizontal,
                            items,
                        }],
                    }];
                }
                if ancestors.is_empty() {
                    draws.extend(items);
                } else {
                    let index = draws.len();
                    draws.extend(items);
                    paint.scopes.push((
                        page_number,
                        index,
                        ancestors.iter().map(|entry| entry.path.clone()).collect(),
                    ));
                }
            }
            paint.pages.push(draws);
        }
        paint
    }
    pub fn update(&mut self, viewports: &Viewports) {
        use wordcraft_layout::display::Draw;
        for (page, index, scopes) in &self.scopes {
            let mut item = self.pages[*page].get_mut(*index);
            for path in scopes {
                let Some(Draw::Clip { items, .. }) = item else {
                    break;
                };
                let Some(Draw::Turned { x, items, .. }) = items.first_mut() else {
                    break;
                };
                *x = -viewports.entries[viewports.index[path]].offset;
                item = items.first_mut();
            }
        }
    }
}
