//! Quick filters for the statement grid, per organisation (`/portal/<org>/statements/filters`).
//!
//! Filters are in groups. On the grid, toggled filters in one group combine with OR and groups
//! with AND (tax year 2024/25 or 2025/26, and rent or utilities). A filter matches a row when
//! every condition it has holds: a date range (on the effective date, or the date when there's
//! none), types (any of), description contains (any of, ignoring case), an amount range. A
//! *yearly* group has no filters of its own: the page offers one per year in the rows, from its
//! start month and day (e.g. tax years from 1 March).
//!
//! The filters are applied in the browser; this keeps their definitions. They're saved as a whole
//! list, like type rules. Types are stored in the list's spelling and follow renames.
//!
//! API: GET (`read-statements`), PUT (`write-statements`).

use crate::db::{self, Db};
use std::{collections::HashSet, sync::Arc};

use axum::{
    extract::{Path as UrlPath, State},
    response::Response,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use statements_core::statement::check_iso_date;

use crate::{
    api::{bad_request, db_error},
    auth::CurrentUser,
    params,
    rbac::{self, Permission},
    statement_types, AppState,
};

const MAX_GROUPS: usize = 20;
const MAX_FILTERS_PER_GROUP: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const MAX_NEEDLES: usize = 20;
const MAX_NEEDLE_CHARS: usize = 200;
const MAX_TYPES: usize = 50;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/orgs/{org}/statement-filters",
        get(get_filters).put(put_filters),
    )
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// Its own filters.
    Filters,
    /// One filter per year, from `start_month` / `start_day`.
    Yearly,
}

impl GroupKind {
    fn as_str(self) -> &'static str {
        match self {
            GroupKind::Filters => "filters",
            GroupKind::Yearly => "yearly",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GroupJson {
    pub name: String,
    pub kind: GroupKind,
    /// Yearly groups: the month (1-12) and day (1-28) each year starts on.
    #[serde(default)]
    pub start_month: Option<u32>,
    #[serde(default)]
    pub start_day: Option<u32>,
    /// `filters` groups only.
    #[serde(default)]
    pub filters: Vec<FilterJson>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct FilterJson {
    pub name: String,
    /// YYYY-MM-DD, inclusive; compared with the effective date (or the date).
    #[serde(default)]
    pub date_from: Option<String>,
    #[serde(default)]
    pub date_to: Option<String>,
    /// Any of these types.
    #[serde(default)]
    pub types: Vec<String>,
    /// The description contains any of these, ignoring case.
    #[serde(default)]
    pub contains: Vec<String>,
    /// Inclusive.
    #[serde(default)]
    pub min_amount: Option<f64>,
    #[serde(default)]
    pub max_amount: Option<f64>,
}

#[derive(Deserialize)]
struct PutBody {
    groups: Vec<GroupJson>,
}

/// GET /api/orgs/{org}/statement-filters — the org's groups, in order.
async fn get_filters(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<Vec<GroupJson>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    Ok(Json(load(&state.db, org.id).await.map_err(db_error)?))
}

/// PUT /api/orgs/{org}/statement-filters — replace every group; returns them as saved.
async fn put_filters(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Json(body): Json<PutBody>,
) -> Result<Json<Vec<GroupJson>>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let groups = validate(&state.db, org.id, body.groups).await?;
    save(&state.db, org.id, &groups).await.map_err(db_error)?;
    tracing::info!(org = %org.slug, groups = groups.len(), user = user.id, "quick filters saved");
    Ok(Json(load(&state.db, org.id).await.map_err(db_error)?))
}

#[derive(Deserialize)]
struct GroupRow {
    id: i64,
    name: String,
    kind: String,
    start_month: Option<u32>,
    start_day: Option<u32>,
}

#[derive(Deserialize)]
struct FilterRow {
    group_id: i64,
    name: String,
    date_from: Option<String>,
    date_to: Option<String>,
    types: String,
    contains: String,
    min_amount: Option<f64>,
    max_amount: Option<f64>,
}

pub async fn load(db: &Db, org_id: i64) -> db::Result<Vec<GroupJson>> {
    let groups: Vec<GroupRow> = db::query_as(
        "SELECT id, name, kind, start_month, start_day FROM statement_filter_groups
         WHERE org_id = ? ORDER BY position, id",
    )
    .bind(org_id)
    .fetch_all(db)
    .await?;
    let filters: Vec<FilterRow> = db::query_as(
        "SELECT group_id, name, date_from, date_to, types, contains, min_amount, max_amount
         FROM statement_quick_filters WHERE org_id = ? ORDER BY position, id",
    )
    .bind(org_id)
    .fetch_all(db)
    .await?;
    let list = |json: &str| serde_json::from_str::<Vec<String>>(json).unwrap_or_default();
    Ok(groups
        .into_iter()
        .map(|g| GroupJson {
            kind: if g.kind == "yearly" {
                GroupKind::Yearly
            } else {
                GroupKind::Filters
            },
            filters: filters
                .iter()
                .filter(|f| f.group_id == g.id)
                .map(|f| FilterJson {
                    name: f.name.clone(),
                    date_from: f.date_from.clone(),
                    date_to: f.date_to.clone(),
                    types: list(&f.types),
                    contains: list(&f.contains),
                    min_amount: f.min_amount,
                    max_amount: f.max_amount,
                })
                .collect(),
            name: g.name,
            start_month: g.start_month,
            start_day: g.start_day,
        })
        .collect())
}

/// One transaction: the org's groups are replaced. Each group's filters find it as the newest
/// group (the batch is the only writer while it runs).
async fn save(db: &Db, org_id: i64, groups: &[GroupJson]) -> db::Result<()> {
    let mut batch = db::Batch::new();
    batch.push(
        "DELETE FROM statement_quick_filters WHERE org_id = ?",
        params![org_id],
    );
    batch.push(
        "DELETE FROM statement_filter_groups WHERE org_id = ?",
        params![org_id],
    );
    for (position, g) in groups.iter().enumerate() {
        batch.push(
            "INSERT INTO statement_filter_groups (org_id, position, name, kind, start_month, start_day)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                org_id,
                position as i64,
                g.name.clone(),
                g.kind.as_str(),
                g.start_month,
                g.start_day,
            ],
        );
        for (position, f) in g.filters.iter().enumerate() {
            batch.push(
                "INSERT INTO statement_quick_filters
                     (org_id, group_id, position, name, date_from, date_to, types, contains,
                      min_amount, max_amount)
                 VALUES (?1, (SELECT max(id) FROM statement_filter_groups WHERE org_id = ?1),
                         ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    org_id,
                    position as i64,
                    f.name.clone(),
                    f.date_from.clone(),
                    f.date_to.clone(),
                    serde_json::to_string(&f.types).expect("strings serialise"),
                    serde_json::to_string(&f.contains).expect("strings serialise"),
                    f.min_amount,
                    f.max_amount,
                ],
            );
        }
    }
    db::run(db, batch).await?;
    Ok(())
}

/// Checked and tidied (names trimmed, types in the list's spelling, blanks dropped), or a 400
/// naming the first problem.
async fn validate(
    db: &Db,
    org_id: i64,
    groups: Vec<GroupJson>,
) -> Result<Vec<GroupJson>, Response> {
    if groups.len() > MAX_GROUPS {
        return Err(bad_request(&format!("At most {MAX_GROUPS} groups.")));
    }
    let mut group_names = HashSet::new();
    let mut checked = Vec::with_capacity(groups.len());
    for (gi, mut g) in groups.into_iter().enumerate() {
        let where_g = format!("Group {}", gi + 1);
        g.name = name(&g.name).map_err(|e| bad_request(&format!("{where_g}: {e}.")))?;
        if !group_names.insert(g.name.to_lowercase()) {
            return Err(bad_request(&format!(
                "{where_g}: there's already a group called {:?}.",
                g.name
            )));
        }
        let where_g = format!("Group {:?}", g.name);
        match g.kind {
            GroupKind::Yearly => {
                let (Some(month), Some(day)) = (g.start_month, g.start_day) else {
                    return Err(bad_request(&format!(
                        "{where_g}: give the day and month each year starts on."
                    )));
                };
                if !(1..=12).contains(&month) || !(1..=28).contains(&day) {
                    return Err(bad_request(&format!(
                        "{where_g}: years start on a day from 1 to 28 of a month from 1 to 12."
                    )));
                }
                g.filters.clear();
            }
            GroupKind::Filters => {
                g.start_month = None;
                g.start_day = None;
                if g.filters.len() > MAX_FILTERS_PER_GROUP {
                    return Err(bad_request(&format!(
                        "{where_g}: at most {MAX_FILTERS_PER_GROUP} filters."
                    )));
                }
                let mut names = HashSet::new();
                let mut filters = Vec::with_capacity(g.filters.len());
                for (fi, f) in std::mem::take(&mut g.filters).into_iter().enumerate() {
                    let f = check_filter(db, org_id, f)
                        .await?
                        .map_err(|e| bad_request(&format!("{where_g}, filter {}: {e}.", fi + 1)))?;
                    if !names.insert(f.name.to_lowercase()) {
                        return Err(bad_request(&format!(
                            "{where_g}: there's already a filter called {:?}.",
                            f.name
                        )));
                    }
                    filters.push(f);
                }
                g.filters = filters;
            }
        }
        checked.push(g);
    }
    Ok(checked)
}

fn name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(format!(
            "give it a name of up to {MAX_NAME_CHARS} characters"
        ));
    }
    Ok(name.to_string())
}

/// The outer error is the database's; the inner one says what's wrong with the filter.
async fn check_filter(
    db: &Db,
    org_id: i64,
    mut f: FilterJson,
) -> Result<Result<FilterJson, String>, Response> {
    let bad = |msg: String| Ok(Err(msg));
    f.name = match name(&f.name) {
        Ok(n) => n,
        Err(e) => return bad(e),
    };
    let date = |d: Option<String>| -> Result<Option<String>, String> {
        match d.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(d) => check_iso_date(d)
                .map(Some)
                .map_err(|_| format!("{d:?} isn't a date")),
        }
    };
    f.date_from = match date(f.date_from.take()) {
        Ok(d) => d,
        Err(e) => return bad(e),
    };
    f.date_to = match date(f.date_to.take()) {
        Ok(d) => d,
        Err(e) => return bad(e),
    };
    if let (Some(from), Some(to)) = (&f.date_from, &f.date_to) {
        if from > to {
            return bad("the start date is after the end date".into());
        }
    }

    if f.types.len() > MAX_TYPES {
        return bad(format!("at most {MAX_TYPES} categories"));
    }
    let mut types = Vec::new();
    for t in f.types.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        match statement_types::canonical_name(db, org_id, t)
            .await
            .map_err(db_error)?
        {
            Some(name) if !types.contains(&name) => types.push(name),
            Some(_) => {}
            None => return bad(format!("{t:?} isn't on the categories list")),
        }
    }
    f.types = types;

    f.contains = f
        .contains
        .iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    if f.contains.len() > MAX_NEEDLES
        || f.contains
            .iter()
            .any(|c| c.chars().count() > MAX_NEEDLE_CHARS)
    {
        return bad(format!(
            "at most {MAX_NEEDLES} texts to look for, of up to {MAX_NEEDLE_CHARS} characters"
        ));
    }

    for amount in [f.min_amount, f.max_amount].into_iter().flatten() {
        if !amount.is_finite() {
            return bad("amounts must be numbers".into());
        }
    }
    if let (Some(min), Some(max)) = (f.min_amount, f.max_amount) {
        if min > max {
            return bad("the minimum amount is above the maximum".into());
        }
    }

    let any_condition = f.date_from.is_some()
        || f.date_to.is_some()
        || !f.types.is_empty()
        || !f.contains.is_empty()
        || f.min_amount.is_some()
        || f.max_amount.is_some();
    if !any_condition {
        return bad("give it at least one condition".into());
    }
    Ok(Ok(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (Db, i64) {
        let db = db::test_db().await;
        let org: i64 = db::query_scalar(
            "INSERT INTO organisations (slug, name) VALUES ('a', 'A') RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        db::query(
            "INSERT INTO statement_types (org_id, name) VALUES (?1, 'Rent'), (?1, 'UTILITIES')",
        )
        .bind(org)
        .execute(&db)
        .await
        .unwrap();
        (db, org)
    }

    fn filter(name: &str) -> FilterJson {
        FilterJson {
            name: name.into(),
            ..Default::default()
        }
    }

    fn groups() -> Vec<GroupJson> {
        vec![
            GroupJson {
                name: " Tax year ".into(),
                kind: GroupKind::Yearly,
                start_month: Some(3),
                start_day: Some(1),
                filters: vec![],
            },
            GroupJson {
                name: "Household".into(),
                kind: GroupKind::Filters,
                start_month: Some(9),
                start_day: None,
                filters: vec![
                    FilterJson {
                        types: vec!["rent".into(), "Utilities".into(), "RENT".into()],
                        ..filter("Rent & utilities")
                    },
                    FilterJson {
                        contains: vec![" WOOLWORTHS ".into(), "".into()],
                        max_amount: Some(0.0),
                        date_from: Some("2025-03-01".into()),
                        ..filter("Groceries")
                    },
                ],
            },
        ]
    }

    async fn message(db: &Db, org: i64, groups: Vec<GroupJson>) -> String {
        let response = validate(db, org, groups).await.unwrap_err();
        let body = axum::body::to_bytes(response.into_body(), 10_000)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn saved_groups_come_back_tidied_and_in_order() {
        let (db, org) = setup().await;
        let checked = validate(&db, org, groups()).await.unwrap();
        save(&db, org, &checked).await.unwrap();
        let loaded = load(&db, org).await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "Tax year");
        assert_eq!(
            (loaded[0].start_month, loaded[0].start_day),
            (Some(3), Some(1))
        );
        // A filters group has no start; types in the list's spelling, once; blanks dropped.
        assert_eq!((loaded[1].start_month, loaded[1].start_day), (None, None));
        assert_eq!(loaded[1].filters[0].types, ["Rent", "UTILITIES"]);
        assert_eq!(loaded[1].filters[1].contains, ["WOOLWORTHS"]);
        assert_eq!(loaded[1].filters[1].max_amount, Some(0.0));

        // Saving again replaces everything.
        save(&db, org, &checked[1..]).await.unwrap();
        let loaded = load(&db, org).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].filters.len(), 2);
    }

    #[tokio::test]
    async fn bad_filters_are_refused() {
        let (db, org) = setup().await;
        let with = |f: FilterJson| {
            let mut g = groups();
            g[1].filters = vec![f];
            g
        };
        assert!(message(&db, org, with(filter("Empty")))
            .await
            .contains("at least one condition"));
        let unknown = FilterJson {
            types: vec!["Nope".into()],
            ..filter("X")
        };
        assert!(message(&db, org, with(unknown))
            .await
            .contains("\\\"Nope\\\" isn't on the categories list"));
        let backwards = FilterJson {
            date_from: Some("2025-03-01".into()),
            date_to: Some("2025-02-28".into()),
            ..filter("X")
        };
        assert!(message(&db, org, with(backwards))
            .await
            .contains("start date is after the end date"));
        let mut twice = groups();
        twice[1].name = "tax YEAR".into();
        assert!(message(&db, org, twice).await.contains("already a group"));
        let mut day = groups();
        day[0].start_day = Some(29);
        assert!(message(&db, org, day).await.contains("day from 1 to 28"));
    }
}
