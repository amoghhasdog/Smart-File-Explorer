use serde::{Deserialize, Serialize};
use sfe_core::Category;

/// Inclusive-ish bounds; `min` is inclusive, `max` is exclusive for dates
/// (end of the period) and inclusive for sizes/durations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Range<T> {
    pub min: Option<T>,
    pub max: Option<T>,
}

impl<T: Copy + PartialOrd> Range<T> {
    pub fn is_empty(&self) -> bool {
        self.min.is_none() && self.max.is_none()
    }

    pub fn merge(&mut self, other: Range<T>) {
        if let Some(m) = other.min {
            self.min = Some(match self.min {
                Some(cur) if cur > m => cur,
                _ => m,
            });
        }
        if let Some(m) = other.max {
            self.max = Some(match self.max {
                Some(cur) if cur < m => cur,
                _ => m,
            });
        }
    }
}

/// Which timestamp a date expression applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateField {
    Modified,
    Created,
    Taken,
}

/// "a lot of blue": `bins` index the 12-bin hue histogram (30° each from
/// red); `min_share` is the summed share (0..=255) those bins must reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorPred {
    pub name: String,
    pub bins: Vec<u8>,
    pub min_share: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    pub categories: Vec<Category>,
    /// Lowercase, no dot.
    pub exts: Vec<String>,
    pub size: Range<u64>,
    pub modified: Range<i64>,
    pub created: Range<i64>,
    pub taken: Range<i64>,
    pub duration_ms: Range<u64>,
    pub min_width: Option<u32>,
    pub min_height: Option<u32>,
    pub color: Option<ColorPred>,
    /// Case-insensitive substrings the path must contain.
    pub path_contains: Vec<String>,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
            && self.exts.is_empty()
            && self.size.is_empty()
            && self.modified.is_empty()
            && self.created.is_empty()
            && self.taken.is_empty()
            && self.duration_ms.is_empty()
            && self.min_width.is_none()
            && self.min_height.is_none()
            && self.color.is_none()
            && self.path_contains.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChipKind {
    Type,
    Ext,
    Size,
    Duration,
    Date,
    Color,
    Dimensions,
    Phrase,
    About,
    Path,
    Terms,
}

/// A human-readable fragment of the interpretation, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chip {
    pub kind: ChipKind,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Query {
    pub raw: String,
    /// Loose keywords (prefix-matched in FTS, also fed to semantic search).
    pub terms: Vec<String>,
    /// Exact phrases (FTS phrase match).
    pub phrases: Vec<String>,
    /// Natural-language description for semantic search, if any.
    pub semantic: Option<String>,
    pub filters: Filters,
    pub chips: Vec<Chip>,
}

impl Query {
    /// `true` when there is nothing to match text against — results are
    /// driven by filters alone.
    pub fn is_filter_only(&self) -> bool {
        self.terms.is_empty() && self.phrases.is_empty() && self.semantic.is_none()
    }

    pub fn is_empty(&self) -> bool {
        self.is_filter_only() && self.filters.is_empty()
    }
}
