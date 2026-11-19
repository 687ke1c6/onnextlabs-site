//! What deleting some rows would do, worked out from the database's own foreign keys.
//!
//! Starting from the rows to delete, it walks *down* to the rows that reference them and
//! follows each foreign key's `ON DELETE` rule, exactly as SQLite will when the delete runs:
//! CASCADE rows are deleted too (and walked in turn), SET NULL rows stay with their link
//! cleared, and RESTRICT / NO ACTION rows block the delete. It never walks *up* (to what a row
//! points at): that's where a generic walk would reach users and organisations.
//!
//! Read-only: it only counts. The delete itself is an ordinary `DELETE` that the foreign keys
//! carry out in the same transaction.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;
use sqlx::{Row, SqliteConnection};

/// One foreign key, from the referencing (child) table's side.
#[derive(Clone, Debug)]
struct ForeignKey {
    child: String,
    /// The child's column holding the reference.
    from: String,
    /// The parent's referenced column (`id` in this schema).
    to: String,
    on_delete: OnDelete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnDelete {
    /// Deleted along with what they reference.
    Delete,
    /// Kept; their reference is cleared.
    Unlink,
    /// Stop the delete while they exist.
    Block,
}

/// One line of the preview: `count` rows of `table` that the delete would affect this way.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Effect {
    pub table: String,
    /// Plain words for the table, e.g. "statement rows".
    pub label: &'static str,
    pub action: OnDelete,
    pub count: i64,
}

#[derive(Serialize, Debug)]
pub struct Impact {
    /// Deleted rows first, then cleared links, then blockers; largest first within each.
    pub effects: Vec<Effect>,
    /// Some rows block the delete (see the `Block` effects).
    pub blocked: bool,
}

/// What people would call a table's rows.
fn label(table: &str) -> &'static str {
    match table {
        "organisations" => "organisation",
        "statement_accounts" => "accounts",
        "org_members" => "memberships (the people's accounts stay)",
        "documents" => "documents (database entries only; files in storage stay)",
        "statement_uploads" => "saved statements",
        "statement_transactions" => "statement rows",
        "statement_upload_transactions" => "links between saved statements and rows",
        "statement_transaction_documents" => "documents attached to rows (the links only)",
        "statement_types" => "statement types",
        "statement_type_rules" => "type rules",
        "users" => "accounts",
        "sessions" => "sign-in sessions",
        "roles" => "roles",
        "role_permissions" => "role permissions",
        "subscribers" => "subscribers",
        _ => "other records",
    }
}

/// Every foreign key in the database, grouped by the table they reference.
async fn foreign_keys(db: &mut SqliteConnection) -> sqlx::Result<HashMap<String, Vec<ForeignKey>>> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '\\_sqlx%' ESCAPE '\\'",
    )
    .fetch_all(&mut *db)
    .await?;
    let mut by_parent: HashMap<String, Vec<ForeignKey>> = HashMap::new();
    for child in tables {
        let rows = sqlx::query(
            "SELECT \"table\", \"from\", \"to\", on_delete FROM pragma_foreign_key_list(?)",
        )
        .bind(&child)
        .fetch_all(&mut *db)
        .await?;
        for row in rows {
            let parent: String = row.try_get("table")?;
            let from: String = row.try_get("from")?;
            let to: Option<String> = row.try_get("to")?;
            let on_delete = match row.try_get::<String, _>("on_delete")?.as_str() {
                "CASCADE" => OnDelete::Delete,
                "SET NULL" | "SET DEFAULT" => OnDelete::Unlink,
                _ => OnDelete::Block, // RESTRICT, NO ACTION
            };
            by_parent.entry(parent).or_default().push(ForeignKey {
                child: child.clone(),
                from,
                to: to.unwrap_or_else(|| "rowid".into()),
                on_delete,
            });
        }
    }
    Ok(by_parent)
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `?, ?, …` for `n` values.
fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// Values of `column` in the rows of `table` with these rowids. Table and column names come
/// from the database's own schema (sqlite_master and its foreign keys), never from a request,
/// which is why the SQL is built here and marked safe.
async fn column_of(
    db: &mut SqliteConnection,
    table: &str,
    column: &str,
    rowids: &[i64],
) -> sqlx::Result<Vec<i64>> {
    let mut out = Vec::new();
    for chunk in rowids.chunks(500) {
        let sql = format!(
            "SELECT {} FROM {} WHERE rowid IN ({})",
            quote(column),
            quote(table),
            placeholders(chunk.len())
        );
        let mut q = sqlx::query_scalar::<_, Option<i64>>(sqlx::AssertSqlSafe(sql));
        for id in chunk {
            q = q.bind(id);
        }
        out.extend(q.fetch_all(&mut *db).await?.into_iter().flatten());
    }
    Ok(out)
}

/// Rowids of `table`'s rows whose `column` is one of `values`.
async fn rows_referencing(
    db: &mut SqliteConnection,
    table: &str,
    column: &str,
    values: &[i64],
) -> sqlx::Result<Vec<i64>> {
    let mut out = Vec::new();
    for chunk in values.chunks(500) {
        let sql = format!(
            "SELECT rowid FROM {} WHERE {} IN ({})",
            quote(table),
            quote(column),
            placeholders(chunk.len())
        );
        let mut q = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql));
        for v in chunk {
            q = q.bind(v);
        }
        out.extend(q.fetch_all(&mut *db).await?);
    }
    Ok(out)
}

/// What deleting these rows (by rowid; `id` is the rowid in this schema) would do. `roots`
/// lists the starting rows per table; they count as deleted.
pub async fn impact(db: &mut SqliteConnection, roots: &[(&str, Vec<i64>)]) -> sqlx::Result<Impact> {
    let fks = foreign_keys(db).await?;
    // Rows already counted, so shared children and cycles are counted once.
    let mut deleted: HashMap<String, HashSet<i64>> = HashMap::new();
    let mut unlinked: HashMap<String, HashSet<i64>> = HashMap::new();
    let mut blocking: HashMap<String, HashSet<i64>> = HashMap::new();

    let mut queue: Vec<(String, Vec<i64>)> = Vec::new();
    for (table, ids) in roots {
        let fresh: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| deleted.entry(table.to_string()).or_default().insert(*id))
            .collect();
        queue.push((table.to_string(), fresh));
    }

    while let Some((table, rowids)) = queue.pop() {
        if rowids.is_empty() {
            continue;
        }
        for fk in fks.get(&table).into_iter().flatten() {
            let keys = column_of(db, &table, &fk.to, &rowids).await?;
            if keys.is_empty() {
                continue;
            }
            let children = rows_referencing(db, &fk.child, &fk.from, &keys).await?;
            match fk.on_delete {
                OnDelete::Delete => {
                    let seen = deleted.entry(fk.child.clone()).or_default();
                    let fresh: Vec<i64> =
                        children.into_iter().filter(|r| seen.insert(*r)).collect();
                    queue.push((fk.child.clone(), fresh));
                }
                OnDelete::Unlink => {
                    unlinked
                        .entry(fk.child.clone())
                        .or_default()
                        .extend(children);
                }
                OnDelete::Block => {
                    blocking
                        .entry(fk.child.clone())
                        .or_default()
                        .extend(children);
                }
            }
        }
    }

    // A row that's deleted anyway isn't also "kept with its link cleared" or "blocking".
    let mut effects = BTreeMap::new();
    for (action, sets) in [
        (OnDelete::Delete, &deleted),
        (OnDelete::Unlink, &unlinked),
        (OnDelete::Block, &blocking),
    ] {
        for (table, rows) in sets {
            let gone = deleted.get(table);
            let count = if action == OnDelete::Delete {
                rows.len()
            } else {
                rows.iter()
                    .filter(|r| !gone.is_some_and(|g| g.contains(r)))
                    .count()
            };
            if count > 0 {
                effects.insert((action, table.clone()), count as i64);
            }
        }
    }
    let mut effects: Vec<Effect> = effects
        .into_iter()
        .map(|((action, table), count)| Effect {
            label: label(&table),
            table,
            action,
            count,
        })
        .collect();
    effects.sort_by(|a, b| a.action.cmp(&b.action).then(b.count.cmp(&a.count)));
    let blocked = effects.iter().any(|e| e.action == OnDelete::Block);
    Ok(Impact { effects, blocked })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn db() -> SqlitePool {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&db)
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&db).await.unwrap();
        db
    }

    async fn exec(db: &SqlitePool, sql: &'static str) {
        sqlx::query(sql).execute(db).await.unwrap();
    }

    fn find<'a>(i: &'a Impact, table: &str, action: OnDelete) -> Option<&'a Effect> {
        i.effects
            .iter()
            .find(|e| e.table == table && e.action == action)
    }

    #[tokio::test]
    async fn follows_each_foreign_keys_rule_downwards_only() {
        let db = db().await;
        exec(
            &db,
            "INSERT INTO users (id, issuer, subject) VALUES (1, 't', '1')",
        )
        .await;
        exec(
            &db,
            "INSERT INTO organisations (id, slug, name) VALUES (1, 'a', 'A'), (2, 'b', 'B')",
        )
        .await;
        exec(
            &db,
            "INSERT INTO org_members (org_id, user_id, role_id) VALUES (1, 1, 2)",
        )
        .await;
        exec(&db, "INSERT INTO documents (id, org_id, folder, filename, size, uploaded_by) VALUES (1, 1, '', 's.csv', 1, 1), (2, 2, '', 'other.pdf', 1, 1)").await;
        exec(
            &db,
            "INSERT INTO statement_accounts (id, org_id, name) VALUES (1, 1, 'Main')",
        )
        .await;
        exec(&db, "INSERT INTO statement_uploads (id, org_id, account_id, format, filename, sha256, rows_parsed, rows_new, rows_updated, document_id, uploaded_by) VALUES (1, 1, 1, 'sb_csv', 's.csv', 'x', 2, 2, 0, 1, 1)").await;
        exec(&db, "INSERT INTO statement_transactions (id, org_id, account_id, txn_id, date, amount, description, currency) VALUES (1, 1, 1, 1, '2027-01-01', '1', 'a', 'ZAR'), (2, 1, 1, 2, '2027-01-02', '2', 'b', 'ZAR')").await;
        exec(&db, "INSERT INTO statement_upload_transactions (upload_id, transaction_id, position) VALUES (1, 1, 0), (1, 2, 1)").await;
        exec(&db, "INSERT INTO statement_transaction_documents (transaction_id, document_id) VALUES (1, 1)").await;
        exec(
            &db,
            "INSERT INTO statement_types (org_id, name) VALUES (1, 'FEES')",
        )
        .await;

        let mut conn = db.acquire().await.unwrap();
        let org = impact(&mut conn, &[("organisations", vec![1])])
            .await
            .unwrap();
        let count = |t, a| find(&org, t, a).map(|e| e.count);
        assert_eq!(count("organisations", OnDelete::Delete), Some(1));
        assert_eq!(count("org_members", OnDelete::Delete), Some(1));
        assert_eq!(count("documents", OnDelete::Delete), Some(1));
        assert_eq!(count("statement_uploads", OnDelete::Delete), Some(1));
        assert_eq!(count("statement_transactions", OnDelete::Delete), Some(2));
        assert_eq!(
            count("statement_upload_transactions", OnDelete::Delete),
            Some(2)
        );
        assert_eq!(
            count("statement_transaction_documents", OnDelete::Delete),
            Some(1)
        );
        assert_eq!(count("statement_types", OnDelete::Delete), Some(1));
        assert_eq!(count("statement_accounts", OnDelete::Delete), Some(1));
        // The upload is deleted anyway, so it isn't also listed as "link cleared".
        assert_eq!(count("statement_uploads", OnDelete::Unlink), None);
        // Never up: the user and the other org are untouched.
        assert!(find(&org, "users", OnDelete::Delete).is_none());
        assert!(!org.blocked);

        // A document on its own: the upload keeps existing with its link cleared.
        let doc = impact(&mut conn, &[("documents", vec![1])]).await.unwrap();
        assert_eq!(
            find(&doc, "statement_uploads", OnDelete::Unlink).map(|e| e.count),
            Some(1)
        );
        assert_eq!(
            find(&doc, "statement_transaction_documents", OnDelete::Delete).map(|e| e.count),
            Some(1)
        );

        // A role someone holds blocks.
        let role = impact(&mut conn, &[("roles", vec![2])]).await.unwrap();
        assert!(role.blocked);
        assert_eq!(
            find(&role, "org_members", OnDelete::Block).map(|e| e.count),
            Some(1)
        );
        drop(conn);

        // And the real delete does what the preview said.
        exec(&db, "DELETE FROM organisations WHERE id = 1").await;
        for (table, left) in [
            ("documents", 1),
            ("statement_transactions", 0),
            ("statement_upload_transactions", 0),
            ("statement_transaction_documents", 0),
            ("statement_types", 0),
            ("org_members", 0),
            ("users", 1),
        ] {
            let n: i64 =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                    .fetch_one(&db)
                    .await
                    .unwrap();
            assert_eq!(n, left, "{table}");
        }
    }
}
