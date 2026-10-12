//! Date expressions → half-open local time ranges `[start, end)`.

use chrono::{DateTime, Datelike, Days, Local, Months, NaiveDate, TimeZone};

use crate::ast::Range;
use crate::tokens::Tok;

/// A period on the calendar, as unix seconds `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Period {
    pub start: i64,
    pub end: i64,
    /// Short human label, e.g. "Jan 2026".
    pub label_len: usize,
}

pub(crate) fn ts(d: NaiveDate) -> i64 {
    Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .or_else(|| {
            Local
                .from_local_datetime(&d.and_hms_opt(1, 0, 0).unwrap())
                .single()
        })
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

const MONTHS: [(&str, u32); 24] = [
    ("january", 1),
    ("jan", 1),
    ("february", 2),
    ("feb", 2),
    ("march", 3),
    ("mar", 3),
    ("april", 4),
    ("apr", 4),
    ("may", 5),
    ("may", 5),
    ("june", 6),
    ("jun", 6),
    ("july", 7),
    ("jul", 7),
    ("august", 8),
    ("aug", 8),
    ("september", 9),
    ("sep", 9),
    ("october", 10),
    ("oct", 10),
    ("november", 11),
    ("nov", 11),
    ("december", 12),
    ("dec", 12),
];

fn month(tok: &str) -> Option<u32> {
    let t = tok.trim_end_matches('.');
    MONTHS.iter().find(|(n, _)| *n == t).map(|(_, m)| *m)
}

fn year(tok: &str) -> Option<i32> {
    let y: i32 = tok.parse().ok()?;
    (1970..=2099).contains(&y).then_some(y)
}

fn day(tok: &str) -> Option<u32> {
    let t = tok
        .trim_end_matches("st")
        .trim_end_matches("nd")
        .trim_end_matches("rd")
        .trim_end_matches("th");
    let d: u32 = t.parse().ok()?;
    (1..=31).contains(&d).then_some(d)
}

pub(crate) fn unit_days(tok: &str) -> Option<Unit> {
    Some(match tok {
        "day" | "days" => Unit::Day,
        "week" | "weeks" => Unit::Week,
        "month" | "months" => Unit::Month,
        "year" | "years" => Unit::Year,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Day,
    Week,
    Month,
    Year,
}

fn minus(d: NaiveDate, n: u32, u: Unit) -> NaiveDate {
    match u {
        Unit::Day => d - Days::new(n as u64),
        Unit::Week => d - Days::new(7 * n as u64),
        Unit::Month => d - Months::new(n),
        Unit::Year => d - Months::new(12 * n),
    }
}

fn period_year(y: i32) -> Period {
    let s = NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
    Period {
        start: ts(s),
        end: ts(NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap()),
        label_len: 0,
    }
}

fn period_month(y: i32, m: u32) -> Period {
    let s = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
    Period {
        start: ts(s),
        end: ts(s + Months::new(1)),
        label_len: 0,
    }
}

fn period_day(d: NaiveDate) -> Period {
    Period {
        start: ts(d),
        end: ts(d + Days::new(1)),
        label_len: 0,
    }
}

/// Try to read a calendar period starting at `toks[i]`. Returns the period
/// and the number of tokens consumed.
pub(crate) fn period_at(toks: &[Tok], i: usize, now: DateTime<Local>) -> Option<(Period, usize)> {
    let today = now.date_naive();
    let t = |k: usize| toks.get(i + k).filter(|t| !t.used).map(|t| t.text.as_str());
    let w0 = t(0)?;

    // today / yesterday
    match w0 {
        "today" => return Some((period_day(today), 1)),
        "yesterday" => return Some((period_day(today - Days::new(1)), 1)),
        _ => {}
    }
    // this|last|past week|month|year  ·  last|past N units  ·  N units ago
    if matches!(w0, "this" | "last" | "past" | "previous") {
        if let Some(u) = t(1).and_then(unit_days) {
            let p = match (w0, u) {
                ("this", Unit::Day) => period_day(today),
                ("this", Unit::Week) => {
                    let start = today - Days::new(today.weekday().num_days_from_monday() as u64);
                    Period {
                        start: ts(start),
                        end: ts(start + Days::new(7)),
                        label_len: 0,
                    }
                }
                ("this", Unit::Month) => period_month(today.year(), today.month()),
                ("this", Unit::Year) => period_year(today.year()),
                (_, Unit::Day) => period_day(today - Days::new(1)),
                (_, Unit::Week) => {
                    let start =
                        today - Days::new(today.weekday().num_days_from_monday() as u64 + 7);
                    Period {
                        start: ts(start),
                        end: ts(start + Days::new(7)),
                        label_len: 0,
                    }
                }
                (_, Unit::Month) => {
                    let m = today - Months::new(1);
                    period_month(m.year(), m.month())
                }
                (_, Unit::Year) => period_year(today.year() - 1),
            };
            return Some((p, 2));
        }
        if w0 != "this"
            && let Some(n) = t(1).and_then(|s| s.parse::<u32>().ok())
            && let Some(u) = t(2).and_then(unit_days)
        {
            let start = minus(today, n, u);
            return Some((
                Period {
                    start: ts(start),
                    end: ts(today + Days::new(1)),
                    label_len: 0,
                },
                3,
            ));
        }
    }
    if let Some(n) = w0
        .parse::<u32>()
        .ok()
        .or((w0 == "a" || w0 == "an").then_some(1))
        && let Some(u) = t(1).and_then(unit_days)
        && t(2) == Some("ago")
    {
        let start = minus(today, n, u);
        return Some((
            Period {
                start: ts(start),
                end: ts(today + Days::new(1)),
                label_len: 0,
            },
            3,
        ));
    }

    // ISO date 2026-01-15
    if let Ok(d) = NaiveDate::parse_from_str(w0, "%Y-%m-%d") {
        return Some((period_day(d), 1));
    }
    // US date 1/15/2026
    if let Ok(d) = NaiveDate::parse_from_str(w0, "%m/%d/%Y") {
        return Some((period_day(d), 1));
    }
    // month [day[,]] [year]  ·  day month [year]
    if let Some(m) = month(w0) {
        if let Some(d) = t(1).and_then(day) {
            if let Some(y) = t(2).and_then(year) {
                if let Some(date) = NaiveDate::from_ymd_opt(y, m, d) {
                    return Some((period_day(date), 3));
                }
            } else if let Some(date) = NaiveDate::from_ymd_opt(today.year(), m, d) {
                return Some((period_day(date), 2));
            }
        }
        if let Some(y) = t(1).and_then(year) {
            return Some((period_month(y, m), 2));
        }
        // bare month: the most recent occurrence
        let y = if m > today.month() {
            today.year() - 1
        } else {
            today.year()
        };
        return Some((period_month(y, m), 1));
    }
    if let Some(d) = day(w0)
        && let Some(m) = t(1).and_then(month)
    {
        if let Some(y) = t(2).and_then(year)
            && let Some(date) = NaiveDate::from_ymd_opt(y, m, d)
        {
            return Some((period_day(date), 3));
        }
        if let Some(date) = NaiveDate::from_ymd_opt(today.year(), m, d) {
            return Some((period_day(date), 2));
        }
    }
    // bare year
    if let Some(y) = year(w0) {
        return Some((period_year(y), 1));
    }
    None
}

/// Apply a qualifier to a period.
pub(crate) fn apply(qual: Qual, p: Period) -> Range<i64> {
    match qual {
        Qual::In => Range {
            min: Some(p.start),
            max: Some(p.end),
        },
        Qual::Before => Range {
            min: None,
            max: Some(p.start),
        },
        Qual::After => Range {
            min: Some(p.end),
            max: None,
        },
        Qual::Since => Range {
            min: Some(p.start),
            max: None,
        },
        Qual::Until => Range {
            min: None,
            max: Some(p.end),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Qual {
    In,
    Before,
    After,
    Since,
    Until,
}

pub(crate) fn qualifier(toks: &[Tok], i: usize) -> Option<(Qual, usize)> {
    let t = |k: usize| toks.get(i + k).filter(|t| !t.used).map(|t| t.text.as_str());
    Some(match (t(0)?, t(1), t(2)) {
        ("before", ..) => (Qual::Before, 1),
        ("until", ..) | ("till", ..) | ("by", ..) | ("through", ..) => (Qual::Until, 1),
        ("prior", Some("to"), _) | ("earlier", Some("than"), _) => (Qual::Before, 2),
        ("older", Some("than"), _) => (Qual::Before, 2),
        ("after", ..) => (Qual::After, 1),
        ("later", Some("than"), _) | ("newer", Some("than"), _) => (Qual::After, 2),
        ("since", ..) => (Qual::Since, 1),
        ("in", ..) | ("during", ..) | ("from", ..) | ("on", ..) => (Qual::In, 1),
        _ => return None,
    })
}

/// `older than 3 months` / `newer than 2 weeks` / `within the last 30 days`.
pub(crate) fn relative_bound(
    toks: &[Tok],
    i: usize,
    now: DateTime<Local>,
) -> Option<(Range<i64>, usize)> {
    let t = |k: usize| toks.get(i + k).filter(|t| !t.used).map(|t| t.text.as_str());
    let today = now.date_naive();
    let (older, skip) = match (t(0)?, t(1)) {
        ("older", Some("than")) => (true, 2),
        ("newer", Some("than")) | ("within", Some("the")) | ("within", Some("last")) => {
            (false, if t(1) == Some("the") { 3 } else { 2 })
        }
        ("within", _) => (false, 1),
        _ => return None,
    };
    let n = t(skip).and_then(|s| {
        s.parse::<u32>()
            .ok()
            .or((s == "a" || s == "an").then_some(1))
    })?;
    let u = t(skip + 1).and_then(unit_days)?;
    let point = ts(minus(today, n, u));
    let r = if older {
        Range {
            min: None,
            max: Some(point),
        }
    } else {
        Range {
            min: Some(point),
            max: None,
        }
    };
    Some((r, skip + 2))
}

pub(crate) fn label(r: &Range<i64>) -> String {
    let fmt = |t: i64| {
        DateTime::from_timestamp(t, 0)
            .map(|d| d.with_timezone(&Local).format("%-d %b %Y").to_string())
            .unwrap_or_default()
    };
    match (r.min, r.max) {
        (Some(a), Some(b)) => format!("{} – {}", fmt(a), fmt(b - 1)),
        (Some(a), None) => format!("since {}", fmt(a)),
        (None, Some(b)) => format!("before {}", fmt(b)),
        (None, None) => String::new(),
    }
}
