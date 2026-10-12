//! Push structured filters down into SQL. Every query selects from
//! `files` aliased `f` with `file_meta` left-joined as `fm`.

use sfe_db::rusqlite::types::Value;
use sfe_query::Filters;

/// `FROM` clause every search query shares.
pub const FROM: &str = "files f LEFT JOIN file_meta fm ON fm.file_id = f.id";

/// Always-on predicate: live, non-directory rows.
pub const BASE: &str = "f.deleted = 0 AND f.kind = 'file'";

/// Build `AND …` fragments plus their bound values. Placeholders are `?`
/// (positional), appended in order, so callers must bind these *after* any
/// of their own leading parameters only if they also use `?` — simplest is
/// to put filter params first; see [`crate::keyword`].
pub fn where_clause(filters: &Filters) -> (String, Vec<Value>) {
    let mut sql = String::new();
    let mut params: Vec<Value> = Vec::new();

    // Type words and extensions are alternatives the user listed: OR them.
    let mut type_parts = Vec::new();
    if !filters.categories.is_empty() {
        type_parts.push(format!(
            "f.category IN ({})",
            marks(filters.categories.len())
        ));
        params.extend(
            filters
                .categories
                .iter()
                .map(|c| Value::from(c.as_str().to_string())),
        );
    }
    if !filters.exts.is_empty() {
        type_parts.push(format!("f.ext IN ({})", marks(filters.exts.len())));
        params.extend(filters.exts.iter().map(|e| Value::from(e.clone())));
    }
    if !type_parts.is_empty() {
        sql.push_str(&format!(" AND ({})", type_parts.join(" OR ")));
    }

    range(
        &mut sql,
        &mut params,
        "f.size",
        filters.size.min.map(|v| v as i64),
        filters.size.max.map(|v| v as i64),
        true,
    );
    range(
        &mut sql,
        &mut params,
        "f.mtime",
        filters.modified.min,
        filters.modified.max,
        false,
    );
    range(
        &mut sql,
        &mut params,
        "f.ctime",
        filters.created.min,
        filters.created.max,
        false,
    );
    range(
        &mut sql,
        &mut params,
        "fm.taken_at",
        filters.taken.min,
        filters.taken.max,
        false,
    );
    range(
        &mut sql,
        &mut params,
        "fm.duration_ms",
        filters.duration_ms.min.map(|v| v as i64),
        filters.duration_ms.max.map(|v| v as i64),
        true,
    );
    if let Some(w) = filters.min_width {
        sql.push_str(" AND fm.width >= ?");
        params.push(Value::from(w as i64));
    }
    if let Some(h) = filters.min_height {
        sql.push_str(" AND fm.height >= ?");
        params.push(Value::from(h as i64));
    }
    if let Some(c) = &filters.color {
        let bins = c
            .bins
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(" AND hue_share(fm.hue_hist, ?) >= ?");
        params.push(Value::from(bins));
        params.push(Value::from(c.min_share as i64));
    }
    for p in &filters.path_contains {
        sql.push_str(" AND instr(lower(f.path), ?) > 0");
        params.push(Value::from(p.to_lowercase()));
    }
    (sql, params)
}

fn range(
    sql: &mut String,
    params: &mut Vec<Value>,
    col: &str,
    min: Option<i64>,
    max: Option<i64>,
    max_inclusive: bool,
) {
    if let Some(m) = min {
        sql.push_str(&format!(" AND {col} >= ?"));
        params.push(Value::from(m));
    }
    if let Some(m) = max {
        sql.push_str(&format!(
            " AND {col} {} ?",
            if max_inclusive { "<=" } else { "<" }
        ));
        params.push(Value::from(m));
    }
}

fn marks(n: usize) -> String {
    vec!["?"; n].join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sfe_core::Category;
    use sfe_query::{ColorPred, Range};

    #[test]
    fn builds_fragments_and_params_in_order() {
        let f = Filters {
            categories: vec![Category::Image],
            exts: vec!["png".into()],
            size: Range {
                min: Some(10),
                max: Some(20),
            },
            modified: Range {
                min: None,
                max: Some(1000),
            },
            color: Some(ColorPred {
                name: "blue".into(),
                bins: vec![7, 8],
                min_share: 102,
            }),
            path_contains: vec!["Downloads".into()],
            ..Default::default()
        };
        let (sql, params) = where_clause(&f);
        assert_eq!(
            sql,
            " AND (f.category IN (?) OR f.ext IN (?)) AND f.size >= ? AND f.size <= ? AND f.mtime < ? AND hue_share(fm.hue_hist, ?) >= ? AND instr(lower(f.path), ?) > 0"
        );
        assert_eq!(params.len(), 8);
        assert_eq!(params[0], Value::from("image".to_string()));
        assert_eq!(params[5], Value::from("7,8".to_string()));
        assert_eq!(params[7], Value::from("downloads".to_string()));
    }

    #[test]
    fn empty_filters_produce_nothing() {
        let (sql, params) = where_clause(&Filters::default());
        assert!(sql.is_empty());
        assert!(params.is_empty());
    }
}
