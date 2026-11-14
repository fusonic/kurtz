//! Database access for PostgreSQL and MySQL.
//!
//! Queries are written once in portable SQL with `$n` placeholders and
//! converted to `?` for MySQL. Parameters must be used once each, in order.
//! This module is the only place that knows about both dialects.

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::{
    mysql::{MySqlPool, MySqlPoolOptions},
    postgres::{PgPool, PgPoolOptions},
    MySql, Postgres, Transaction,
};

use crate::cache::Resolved;

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub id: i64,
    pub slug: String,
    pub target_url: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// Tags, sorted.
    pub tags: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct LinkRow {
    id: i64,
    slug: String,
    target_url: String,
    enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

impl LinkRow {
    fn into_link(self, tags: Vec<String>) -> Link {
        Link {
            id: self.id,
            slug: self.slug,
            target_url: self.target_url,
            enabled: self.enabled,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_used_at: self.last_used_at,
            tags,
        }
    }
}

/// Data for a new link.
#[derive(Debug, Clone, PartialEq)]
pub struct NewLink {
    pub slug: String,
    pub target_url: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub tags: Vec<String>,
}

/// Filters for the link list.
#[derive(Debug, Clone, Default)]
pub struct ListFilter<'a> {
    /// Matches slug, target or tag (case-insensitive substring).
    pub search: Option<&'a str>,
    /// Only links with this tag.
    pub tag: Option<&'a str>,
}

#[derive(Clone)]
pub enum Db {
    Pg(PgPool),
    My(MySqlPool),
}

pub enum Tx {
    Pg(Transaction<'static, Postgres>),
    My(Transaction<'static, MySql>),
}

/// Converts `$1, $2, …` placeholders to MySQL's `?`.
pub fn to_mysql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek().is_some_and(|n| n.is_ascii_digit()) {
            while chars.peek().is_some_and(|n| n.is_ascii_digit()) {
                chars.next();
            }
            out.push('?');
        } else {
            out.push(c);
        }
    }
    out
}

/// Runs the same body against whichever pool is configured.
macro_rules! on_pool {
    ($db:expr, |$pool:ident, $sql:ident| $text:expr => $body:expr) => {
        match $db {
            Db::Pg($pool) => {
                let $sql: String = $text.to_string();
                $body
            }
            Db::My($pool) => {
                let $sql: String = to_mysql($text);
                $body
            }
        }
    };
}

/// Runs the same body inside whichever transaction is open.
macro_rules! on_tx {
    ($tx:expr, |$conn:ident, $sql:ident| $text:expr => $body:expr) => {
        match $tx {
            Tx::Pg(t) => {
                let $conn = &mut **t;
                let $sql: String = $text.to_string();
                $body
            }
            Tx::My(t) => {
                let $conn = &mut **t;
                let $sql: String = to_mysql($text);
                $body
            }
        }
    };
}

static PG_MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
static MYSQL_MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/mysql");

const LINK_COLUMNS: &str = "id, slug, target_url, enabled, created_at, updated_at, last_used_at";

fn now() -> DateTime<Utc> {
    use chrono::SubsecRound;
    Utc::now().trunc_subsecs(6)
}

fn group_tags(rows: Vec<(i64, String)>) -> HashMap<i64, Vec<String>> {
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for (id, t) in rows {
        map.entry(id).or_default().push(t);
    }
    for v in map.values_mut() {
        v.sort();
    }
    map
}

/// Escapes `%`, `_` and `\` for a LIKE pattern.
fn like_pattern(q: &str) -> String {
    let mut p = String::from("%");
    for c in q.to_lowercase().chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

impl Db {
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        let opts_timeout = Duration::from_secs(5);
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Ok(Db::Pg(
                PgPoolOptions::new()
                    .max_connections(5)
                    .acquire_timeout(opts_timeout)
                    .connect(url)
                    .await?,
            ))
        } else if url.starts_with("mysql://") || url.starts_with("mariadb://") {
            let url = url.replacen("mariadb://", "mysql://", 1);
            Ok(Db::My(
                MySqlPoolOptions::new()
                    .max_connections(5)
                    .acquire_timeout(opts_timeout)
                    .after_connect(|conn, _| {
                        Box::pin(async move {
                            sqlx::query("SET time_zone = '+00:00'").execute(conn).await?;
                            Ok(())
                        })
                    })
                    .connect(&url)
                    .await?,
            ))
        } else {
            Err(sqlx::Error::Configuration(
                "DATABASE_URL must start with postgres:// or mysql://".into(),
            ))
        }
    }

    pub fn backend(&self) -> &'static str {
        match self {
            Db::Pg(_) => "postgres",
            Db::My(_) => "mysql",
        }
    }

    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        match self {
            Db::Pg(p) => PG_MIGRATIONS.run(p).await,
            Db::My(p) => MYSQL_MIGRATIONS.run(p).await,
        }
    }

    /// Ok when the database answers and every migration embedded in this
    /// binary has been applied successfully.
    pub async fn check_migrated(&self) -> Result<(), String> {
        let expected: Vec<i64> = match self {
            Db::Pg(_) => PG_MIGRATIONS.iter().map(|m| m.version).collect(),
            Db::My(_) => MYSQL_MIGRATIONS.iter().map(|m| m.version).collect(),
        };
        let applied: Vec<i64> = on_pool!(self, |p, sql| "SELECT version FROM _sqlx_migrations WHERE success = $1" => {
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(true).fetch_all(p).await
        })
        .map_err(|e| format!("database not ready: {e}"))?;
        let pending = expected.iter().filter(|v| !applied.contains(v)).count();
        if pending > 0 {
            return Err(format!("{pending} database migration(s) pending"));
        }
        Ok(())
    }

    pub async fn begin(&self) -> Result<Tx, sqlx::Error> {
        Ok(match self {
            Db::Pg(p) => Tx::Pg(p.begin().await?),
            Db::My(p) => Tx::My(p.begin().await?),
        })
    }

    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        on_pool!(self, |p, sql| "SELECT 1" => {
            sqlx::query(sqlx::AssertSqlSafe(sql)).execute(p).await.map(|_| ())
        })
    }

    /// All links, newest first, optionally filtered.
    pub async fn list(&self, filter: ListFilter<'_>) -> Result<Vec<Link>, sqlx::Error> {
        let search = filter.search.map(str::trim).filter(|s| !s.is_empty()).map(like_pattern);
        let tag = filter
            .tag
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase);
        let mut conditions = Vec::new();
        let mut n = 0;
        let mut next = || {
            n += 1;
            format!("${n}")
        };
        if search.is_some() {
            let (a, b, c) = (next(), next(), next());
            conditions.push(format!(
                "(LOWER(slug) LIKE {a} OR LOWER(target_url) LIKE {b} \
                 OR id IN (SELECT link_id FROM link_tags WHERE tag LIKE {c}))"
            ));
        }
        if tag.is_some() {
            let t = next();
            conditions.push(format!("id IN (SELECT link_id FROM link_tags WHERE tag = {t})"));
        }
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        let text = format!("SELECT {LINK_COLUMNS} FROM links{where_clause} ORDER BY created_at DESC, id DESC");
        let rows: Vec<LinkRow> = on_pool!(self, |p, sql| text.as_str() => {{
            let mut q = sqlx::query_as::<_, LinkRow>(sqlx::AssertSqlSafe(sql));
            if let Some(pat) = &search {
                q = q.bind(pat.clone()).bind(pat.clone()).bind(pat.clone());
            }
            if let Some(t) = &tag {
                q = q.bind(t.clone());
            }
            q.fetch_all(p).await?
        }});
        let tag_rows: Vec<(i64, String)> = on_pool!(self, |p, sql| "SELECT link_id, tag FROM link_tags" => {
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_all(p).await?
        });
        let mut tags = group_tags(tag_rows);
        Ok(rows
            .into_iter()
            .map(|r| {
                let t = tags.remove(&r.id).unwrap_or_default();
                r.into_link(t)
            })
            .collect())
    }

    /// All tags in use with the number of links carrying each, sorted by name.
    pub async fn tags(&self) -> Result<Vec<(String, i64)>, sqlx::Error> {
        on_pool!(self, |p, sql| "SELECT tag, COUNT(*) FROM link_tags GROUP BY tag ORDER BY tag" => {
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_all(p).await
        })
    }

    async fn tags_of(&self, id: i64) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(i64, String)> = on_pool!(self, |p, sql| "SELECT link_id, tag FROM link_tags WHERE link_id = $1" => {
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_all(p).await?
        });
        Ok(group_tags(rows).remove(&id).unwrap_or_default())
    }

    pub async fn get(&self, id: i64) -> Result<Option<Link>, sqlx::Error> {
        let text = format!("SELECT {LINK_COLUMNS} FROM links WHERE id = $1");
        let row: Option<LinkRow> = on_pool!(self, |p, sql| text.as_str() => {
            sqlx::query_as::<_, LinkRow>(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(p).await?
        });
        let Some(row) = row else { return Ok(None) };
        let t = self.tags_of(id).await?;
        Ok(Some(row.into_link(t)))
    }

    /// Finds the link for `slug`.
    pub async fn resolve(&self, slug: &str) -> Result<Option<Resolved>, sqlx::Error> {
        let row: Option<(i64, String, bool)> = on_pool!(self, |p, sql|
            "SELECT id, target_url, enabled FROM links WHERE slug = $1" => {
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(slug).fetch_optional(p).await?
        });
        Ok(row.map(|(link_id, target, enabled)| Resolved {
            link_id,
            target,
            enabled,
        }))
    }

    /// Links with exactly this target (via its hash).
    pub async fn by_target_hash(&self, hash: &str) -> Result<Vec<Link>, sqlx::Error> {
        let text = format!("SELECT {LINK_COLUMNS} FROM links WHERE target_hash = $1 ORDER BY id");
        let rows: Vec<LinkRow> = on_pool!(self, |p, sql| text.as_str() => {
            sqlx::query_as::<_, LinkRow>(sqlx::AssertSqlSafe(sql)).bind(hash).fetch_all(p).await?
        });
        let mut out = Vec::new();
        for r in rows {
            let id = r.id;
            let t = self.tags_of(id).await?;
            out.push(r.into_link(t));
        }
        Ok(out)
    }

    /// Writes last-used timestamps in one transaction.
    pub async fn flush_last_used(&self, entries: &[(i64, DateTime<Utc>)]) -> Result<(), sqlx::Error> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut tx = self.begin().await?;
        for (id, at) in entries {
            on_tx!(&mut tx, |c, sql| "UPDATE links SET last_used_at = $1 WHERE id = $2" => {
                sqlx::query(sqlx::AssertSqlSafe(sql)).bind(*at).bind(*id).execute(&mut *c).await.map(|_| ())?
            });
        }
        tx.commit().await
    }
}

impl Tx {
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        match self {
            Tx::Pg(t) => t.commit().await,
            Tx::My(t) => t.commit().await,
        }
    }

    pub async fn rollback(self) -> Result<(), sqlx::Error> {
        match self {
            Tx::Pg(t) => t.rollback().await,
            Tx::My(t) => t.rollback().await,
        }
    }

    /// The link that already uses `slug`, if any. `exclude` skips one link id
    /// (the one being edited).
    pub async fn slug_owner(&mut self, slug: &str, exclude: Option<i64>) -> Result<Option<i64>, sqlx::Error> {
        let exclude = exclude.unwrap_or(0);
        let row: Option<(i64,)> = on_tx!(self, |c, sql| "SELECT id FROM links WHERE slug = $1 AND id <> $2" => {
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(slug).bind(exclude).fetch_optional(&mut *c).await?
        });
        Ok(row.map(|r| r.0))
    }

    pub async fn count_target(&mut self, hash: &str) -> Result<i64, sqlx::Error> {
        let row: (i64,) = on_tx!(self, |c, sql| "SELECT COUNT(*) FROM links WHERE target_hash = $1" => {
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(hash).fetch_one(&mut *c).await?
        });
        Ok(row.0)
    }

    pub async fn insert(&mut self, link: &NewLink) -> Result<i64, sqlx::Error> {
        let hash = crate::validate::target_hash(&link.target_url);
        let id = match self {
            Tx::Pg(t) => {
                let row: (i64,) = sqlx::query_as(
                    "INSERT INTO links (slug, target_url, target_hash, enabled, created_at, updated_at, last_used_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
                )
                .bind(&link.slug)
                .bind(&link.target_url)
                .bind(&hash)
                .bind(link.enabled)
                .bind(link.created_at)
                .bind(link.updated_at)
                .bind(link.last_used_at)
                .fetch_one(&mut **t)
                .await?;
                row.0
            }
            Tx::My(t) => {
                let res = sqlx::query(
                    "INSERT INTO links (slug, target_url, target_hash, enabled, created_at, updated_at, last_used_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&link.slug)
                .bind(&link.target_url)
                .bind(&hash)
                .bind(link.enabled)
                .bind(link.created_at)
                .bind(link.updated_at)
                .bind(link.last_used_at)
                .execute(&mut **t)
                .await?;
                res.last_insert_id() as i64
            }
        };
        self.set_tags(id, &link.tags).await?;
        Ok(id)
    }

    /// Replaces the tags of a link.
    pub async fn set_tags(&mut self, id: i64, tags: &[String]) -> Result<(), sqlx::Error> {
        on_tx!(self, |c, sql| "DELETE FROM link_tags WHERE link_id = $1" => {
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).execute(&mut *c).await.map(|_| ())?
        });
        for t in tags {
            on_tx!(self, |c, sql| "INSERT INTO link_tags (link_id, tag) VALUES ($1, $2)" => {
                sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).bind(t).execute(&mut *c).await.map(|_| ())?
            });
        }
        Ok(())
    }

    pub async fn update(&mut self, id: i64, target: &str, enabled: bool, tags: &[String]) -> Result<(), sqlx::Error> {
        let hash = crate::validate::target_hash(target);
        let ts = now();
        on_tx!(self, |c, sql| "UPDATE links SET target_url = $1, target_hash = $2, enabled = $3, updated_at = $4 WHERE id = $5" => {
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(target).bind(&hash).bind(enabled).bind(ts).bind(id).execute(&mut *c).await.map(|_| ())?
        });
        self.set_tags(id, tags).await
    }

    pub async fn set_enabled(&mut self, id: i64, enabled: bool) -> Result<(), sqlx::Error> {
        let ts = now();
        on_tx!(self, |c, sql| "UPDATE links SET enabled = $1, updated_at = $2 WHERE id = $3" => {
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(enabled).bind(ts).bind(id).execute(&mut *c).await.map(|_| ())?
        });
        Ok(())
    }

    pub async fn delete(&mut self, id: i64) -> Result<(), sqlx::Error> {
        on_tx!(self, |c, sql| "DELETE FROM link_tags WHERE link_id = $1" => {
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).execute(&mut *c).await.map(|_| ())?
        });
        on_tx!(self, |c, sql| "DELETE FROM links WHERE id = $1" => {
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).execute(&mut *c).await.map(|_| ())?
        });
        Ok(())
    }
}

/// Current time with the precision both databases store (microseconds).
pub fn timestamp() -> DateTime<Utc> {
    now()
}
