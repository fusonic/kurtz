//! CSV export and import.
//!
//! Columns: `slug, target_url, tags, enabled, created_at, updated_at,
//! last_used_at`. `tags` are semicolon-separated.
//! Dates are RFC 3339 in UTC.

use chrono::{DateTime, SecondsFormat, SubsecRound, Utc};

use crate::{
    db::{Db, Link, NewLink},
    validate,
};

pub const HEADER: [&str; 7] = [
    "slug",
    "target_url",
    "tags",
    "enabled",
    "created_at",
    "updated_at",
    "last_used_at",
];
pub const MAX_ROWS: usize = 50_000;
pub const MAX_BYTES: usize = 5 * 1024 * 1024;

fn fmt_date(d: &DateTime<Utc>) -> String {
    d.to_rfc3339_opts(SecondsFormat::Micros, true)
}

pub fn export(links: &[Link]) -> Result<Vec<u8>, csv::Error> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(HEADER)?;
    for l in links {
        w.write_record([
            l.slug.as_str(),
            l.target_url.as_str(),
            &l.tags.join(";"),
            if l.enabled { "true" } else { "false" },
            &fmt_date(&l.created_at),
            &fmt_date(&l.updated_at),
            &l.last_used_at.as_ref().map(fmt_date).unwrap_or_default(),
        ])?;
    }
    w.into_inner().map_err(|e| e.into_error().into())
}

/// A column Kurtz imports. `names` are the accepted header names as lists of
/// words; matching ignores case, `_`, `-` and spaces, so `short_code`,
/// `shortCode` and `ShortCode` are the same header.
pub struct Column {
    pub key: &'static str,
    pub required: bool,
    /// What the column holds, shown next to its name.
    pub hint: &'static str,
    names: &'static [&'static [&'static str]],
}

pub const COLUMNS: [Column; 7] = [
    Column {
        key: "slug",
        required: true,
        hint: "unique, case-insensitive",
        names: &[&["slug"], &["code"], &["short", "code"], &["alias"], &["key"]],
    },
    Column {
        key: "target_url",
        required: true,
        hint: "http or https URL",
        names: &[
            &["target", "url"],
            &["target"],
            &["long", "url"],
            &["url"],
            &["destination"],
            &["original", "url"],
        ],
    },
    Column {
        key: "tags",
        required: false,
        hint: "letters, digits and _; any other character separates tags",
        names: &[&["tags"], &["tag"]],
    },
    Column {
        key: "enabled",
        required: false,
        hint: "true or false, default true",
        names: &[&["enabled"], &["active"]],
    },
    Column {
        key: "created_at",
        required: false,
        hint: "RFC 3339 date",
        names: &[&["created", "at"], &["created"], &["date", "created"]],
    },
    Column {
        key: "updated_at",
        required: false,
        hint: "RFC 3339 date",
        names: &[&["updated", "at"], &["updated"]],
    },
    Column {
        key: "last_used_at",
        required: false,
        hint: "RFC 3339 date",
        names: &[&["last", "used", "at"], &["last", "used"], &["last", "visit"]],
    },
];

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

impl Column {
    /// Every accepted header name in snake_case, kebab-case, camelCase,
    /// PascalCase and UPPER_SNAKE_CASE (single words: lower case, capitalised
    /// and upper case), for display.
    pub fn headers(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        for words in self.names {
            if let [one] = words {
                out.push(one.to_string());
                out.push(capitalize(one));
                out.push(one.to_uppercase());
                continue;
            }
            let pascal: String = words.iter().map(|w| capitalize(w)).collect();
            let camel = format!(
                "{}{}",
                words[0],
                words[1..].iter().map(|w| capitalize(w)).collect::<String>()
            );
            out.push(words.join("_"));
            out.push(words.join("-"));
            out.push(camel);
            out.push(pascal);
            out.push(words.join("_").to_uppercase());
        }
        out.join(", ")
    }

    fn matches(&self, header: &str) -> bool {
        let key: String = header
            .chars()
            .filter(|c| !matches!(c, '_' | '-' | ' '))
            .collect::<String>()
            .to_lowercase();
        self.names.iter().any(|words| words.concat() == key)
    }
}

/// The Kurtz column a header belongs to, if any.
fn column_for(header: &str) -> Option<&'static str> {
    COLUMNS.iter().find(|c| c.matches(header)).map(|c| c.key)
}

/// Tags of the tags cell: every run of `a-zA-Z0-9_` is a tag, anything else
/// separates them.
fn import_tags(cell: &str) -> Result<Vec<String>, String> {
    let found: Vec<&str> = cell
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .collect();
    validate::tags(&found.join(","))
}

/// A parsed, valid CSV file.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub rows: Vec<ImportRow>,
    /// Header names that were not recognised and are ignored.
    pub ignored_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRow {
    pub line: u64,
    pub link: NewLink,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowError {
    pub line: u64,
    pub message: String,
}

fn parse_date(s: &str) -> Result<Option<DateTime<Utc>>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    DateTime::parse_from_rfc3339(s)
        .map(|d| Some(d.with_timezone(&Utc).trunc_subsecs(6)))
        .map_err(|_| format!("“{s}” is not an RFC 3339 date"))
}

fn parse_enabled(s: &str) -> Result<bool, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        other => Err(format!("enabled must be true or false, got “{other}”")),
    }
}

/// Parses and validates a CSV file. Either every row is valid, or the errors
/// are returned with their line numbers.
pub fn parse(bytes: &[u8]) -> Result<Parsed, Vec<RowError>> {
    let err = |line: u64, message: String| vec![RowError { line, message }];
    if bytes.len() > MAX_BYTES {
        return Err(err(0, format!("File is larger than {} MB.", MAX_BYTES / 1024 / 1024)));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(true)
        .from_reader(bytes);

    let raw_headers: Vec<String> = match reader.headers() {
        Ok(h) => h.iter().map(String::from).collect(),
        Err(e) => return Err(err(1, format!("Could not read the header row: {e}"))),
    };
    let headers: Vec<Option<&'static str>> = raw_headers.iter().map(|h| column_for(h)).collect();
    let ignored: Vec<String> = raw_headers
        .iter()
        .zip(&headers)
        .filter(|(_, c)| c.is_none())
        .map(|(h, _)| h.clone())
        .collect();
    // Each column may be given only once, so there is never a doubt which one counts.
    let mut clashes = Vec::new();
    for c in &COLUMNS {
        let matching: Vec<&str> = raw_headers
            .iter()
            .zip(&headers)
            .filter(|(_, k)| **k == Some(c.key))
            .map(|(h, _)| h.as_str())
            .collect();
        if matching.len() > 1 {
            clashes.push(RowError {
                line: 1,
                message: format!(
                    "Several columns match {}: {}. Keep only one.",
                    c.key,
                    matching.join(", ")
                ),
            });
        }
    }
    if !clashes.is_empty() {
        return Err(clashes);
    }
    let col = |name: &str| headers.iter().position(|h| *h == Some(name));
    let (Some(c_slug), Some(c_target)) = (col("slug"), col("target_url")) else {
        return Err(err(
            1,
            "Header row must contain the required columns: slug and target_url.".into(),
        ));
    };
    let (c_tags, c_enabled, c_created, c_updated, c_used) = (
        col("tags"),
        col("enabled"),
        col("created_at"),
        col("updated_at"),
        col("last_used_at"),
    );

    let now = crate::db::timestamp();
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for record in reader.records() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                let line = e.position().map(|p| p.line()).unwrap_or(0);
                errors.push(RowError {
                    line,
                    message: format!("Malformed row: {e}"),
                });
                continue;
            }
        };
        let line = record.position().map(|p| p.line()).unwrap_or(0);
        if rows.len() + errors.len() >= MAX_ROWS {
            errors.push(RowError {
                line,
                message: format!("More than {MAX_ROWS} rows."),
            });
            break;
        }
        let get = |c: Option<usize>| c.and_then(|i| record.get(i)).unwrap_or("");
        let result = (|| -> Result<NewLink, String> {
            let slug = validate::import_slug(get(Some(c_slug)))?;
            let target_url = validate::target_url(get(Some(c_target)))?;
            let tags = import_tags(get(c_tags))?;
            let enabled = parse_enabled(get(c_enabled))?;
            let created_at = parse_date(get(c_created))?.unwrap_or(now);
            let updated_at = parse_date(get(c_updated))?.unwrap_or(created_at);
            let last_used_at = parse_date(get(c_used))?;
            Ok(NewLink {
                tags,
                slug,
                target_url,
                enabled,
                created_at,
                updated_at,
                last_used_at,
            })
        })();
        match result {
            Ok(link) => rows.push(ImportRow { line, link }),
            Err(message) => errors.push(RowError { line, message }),
        }
    }
    if errors.is_empty() {
        Ok(Parsed {
            rows,
            ignored_columns: ignored,
        })
    } else {
        Err(errors)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    Skip,
    Overwrite,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ImportReport {
    pub created: usize,
    pub skipped: usize,
    pub overwritten: usize,
    pub duplicate_targets: usize,
    pub errors: Vec<RowError>,
    pub committed: bool,
    /// Rows not imported because their slug is taken (with policy "skip").
    pub skipped_rows: Vec<RowError>,
}

/// Imports rows in one transaction. With `commit = false` nothing is written
/// (preview). Any database error rolls everything back.
pub async fn import(
    db: &Db,
    rows: &[ImportRow],
    on_conflict: OnConflict,
    commit: bool,
) -> Result<ImportReport, sqlx::Error> {
    let mut report = ImportReport::default();
    let mut tx = db.begin().await?;
    // link id → CSV line, for links created by this import
    let mut from_line: std::collections::HashMap<i64, u64> = std::collections::HashMap::new();
    for row in rows {
        if let Some(existing) = tx.slug_owner(&row.link.slug, None).await? {
            match on_conflict {
                OnConflict::Skip => {
                    report.skipped += 1;
                    let by = match from_line.get(&existing) {
                        Some(line) => format!("line {line} of this file"),
                        None => "an existing link".to_string(),
                    };
                    report.skipped_rows.push(RowError {
                        line: row.line,
                        message: format!("/{} is already used by {by}.", row.link.slug),
                    });
                    continue;
                }
                OnConflict::Overwrite => {
                    tx.delete(existing).await?;
                    report.overwritten += 1;
                }
            }
        } else {
            report.created += 1;
        }
        let hash = validate::target_hash(&row.link.target_url);
        if tx.count_target(&hash).await? > 0 {
            report.duplicate_targets += 1;
        }
        let id = tx.insert(&row.link).await?;
        from_line.insert(id, row.line);
    }
    if commit {
        tx.commit().await?;
        report.committed = true;
    } else {
        tx.rollback().await?;
    }
    Ok(report)
}
