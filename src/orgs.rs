//! Organisations: the scope every portal feature lives under.
//!
//! Admins can see and act on every org ("god mode"); anyone else only on orgs they're a
//! member of, limited by their role there (see `rbac`). Each org's files live in
//! `$DATA_DIR/orgs/<slug>/`.

use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::Response,
    routing::{get, patch},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    api::{bad_request, db_error, error, io_error, not_found},
    auth::{Admin, CurrentUser},
    rbac::{self, Permission, Perms},
    AppState,
};

/// First-level portal paths that can't be org slugs (`/portal/settings` etc.).
const RESERVED_SLUGS: &[&str] = &[
    "access",
    "account",
    "admin",
    "api",
    "login",
    "logout",
    "manage",
    "new",
    "organisations",
    "orgs",
    "portal",
    "roles",
    "settings",
    "users",
];
const MAX_SLUG_LEN: usize = 63;
const MAX_NAME_CHARS: usize = 100;

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Org {
    pub id: i64,
    pub slug: String,
    pub name: String,
}

impl Org {
    /// `$DATA_DIR/orgs/<slug>`: everything stored for this org goes under here.
    pub fn dir(&self, data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("orgs").join(&self.slug)
    }
}

/// SQL expression: comma-separated permissions of role `r` (null if none).
macro_rules! role_permissions_csv {
    () => {
        "(SELECT group_concat(p.permission) FROM role_permissions p WHERE p.role_id = r.id)"
    };
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/orgs", get(my_orgs))
        .route("/api/manage/orgs", get(manageable_orgs).post(create_org))
        .route(
            "/api/orgs/{org}/members",
            get(list_members).post(add_member),
        )
        .route(
            "/api/orgs/{org}/members/{user_id}",
            patch(change_role).delete(remove_member),
        )
        .route("/api/users", get(list_users))
}

/// The org with this slug, or a 404.
pub async fn find(db: &SqlitePool, slug: &str) -> Result<Org, Response> {
    sqlx::query_as("SELECT id, slug, name FROM organisations WHERE slug = ?")
        .bind(slug)
        .fetch_optional(db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Organisation not found."))
}

#[derive(Serialize)]
struct RoleRef {
    id: i64,
    name: String,
}

#[derive(Serialize)]
struct MyOrg {
    id: i64,
    slug: String,
    name: String,
    /// The user's role here; null when an admin opens an org they aren't a member of.
    role: Option<RoleRef>,
    /// What the user may do here (everything for admins). The UI uses this to show or
    /// hide sections; the API checks again on every request.
    permissions: Perms,
}

/// GET /api/orgs — orgs the signed-in user can open (all of them for admins), with their
/// role and permissions in each.
async fn my_orgs(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<MyOrg>>, Response> {
    type Row = (
        i64,
        String,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = if user.is_admin() {
        sqlx::query_as(concat!(
            "SELECT o.id, o.slug, o.name, r.id, r.name, ",
            role_permissions_csv!(),
            " FROM organisations o
             LEFT JOIN org_members m ON m.org_id = o.id AND m.user_id = ?
             LEFT JOIN roles r ON r.id = m.role_id
             ORDER BY o.name COLLATE NOCASE"
        ))
    } else {
        sqlx::query_as(concat!(
            "SELECT o.id, o.slug, o.name, r.id, r.name, ",
            role_permissions_csv!(),
            " FROM organisations o
             JOIN org_members m ON m.org_id = o.id
             JOIN roles r ON r.id = m.role_id
             WHERE m.user_id = ?
             ORDER BY o.name COLLATE NOCASE"
        ))
    }
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;

    let orgs = rows
        .into_iter()
        .map(|(id, slug, name, role_id, role_name, csv)| MyOrg {
            id,
            slug,
            name,
            role: role_id
                .zip(role_name)
                .map(|(id, name)| RoleRef { id, name }),
            permissions: if user.is_admin() {
                Perms::all()
            } else {
                Perms::from_csv(csv.as_deref())
            },
        })
        .collect();
    Ok(Json(orgs))
}

#[derive(Serialize, sqlx::FromRow)]
struct OrgSummary {
    id: i64,
    slug: String,
    name: String,
    created_at: String,
    member_count: i64,
}

/// GET /api/manage/orgs — orgs the user can manage (`manage-orgs`; every org for admins),
/// with member counts.
async fn manageable_orgs(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<OrgSummary>>, Response> {
    let orgs = sqlx::query_as(
        "SELECT o.id, o.slug, o.name, o.created_at,
                (SELECT count(*) FROM org_members m WHERE m.org_id = o.id) AS member_count
         FROM organisations o
         WHERE ? OR EXISTS (
             SELECT 1 FROM org_members m JOIN role_permissions p ON p.role_id = m.role_id
             WHERE m.org_id = o.id AND m.user_id = ? AND p.permission = ?)
         ORDER BY o.name COLLATE NOCASE",
    )
    .bind(user.is_admin())
    .bind(user.id)
    .bind(Permission::ManageOrgs.key())
    .fetch_all(&state.db)
    .await
    .map_err(db_error)?;
    Ok(Json(orgs))
}

#[derive(Deserialize)]
struct NewOrg {
    name: String,
    /// Optional; derived from the name when missing or blank.
    #[serde(default)]
    slug: Option<String>,
}

/// POST /api/manage/orgs — create an org and its folder. Admins only.
async fn create_org(
    Admin(admin): Admin,
    State(state): State<Arc<AppState>>,
    Json(body): Json<NewOrg>,
) -> Result<(StatusCode, Json<Org>), Response> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(bad_request(
            "Give the organisation a name of up to 100 characters.",
        ));
    }
    let slug = match body
        .slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_owned(),
        None => slugify(name),
    };
    validate_slug(&slug).map_err(bad_request)?;

    let inserted = sqlx::query_as::<_, Org>(
        "INSERT INTO organisations (slug, name) VALUES (?, ?) RETURNING id, slug, name",
    )
    .bind(&slug)
    .bind(name)
    .fetch_one(&state.db)
    .await;
    let org = match inserted {
        Ok(org) => org,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(error(
                StatusCode::CONFLICT,
                "That URL name is already taken by another organisation.",
            ))
        }
        Err(e) => return Err(db_error(e)),
    };

    let dir = org
        .dir(&state.data_dir)
        .join(crate::documents::DOCUMENTS_FOLDER);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        let _ = sqlx::query("DELETE FROM organisations WHERE id = ?")
            .bind(org.id)
            .execute(&state.db)
            .await;
        return Err(io_error(e));
    }
    tracing::info!(org = %org.slug, admin = admin.id, "organisation created");
    Ok((StatusCode::CREATED, Json(org)))
}

#[derive(Serialize, sqlx::FromRow)]
struct Member {
    user_id: i64,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
    role_id: i64,
    role_name: String,
    added_at: String,
}

/// The org, if the user may manage its members, plus the user's own permissions there
/// (which cap the roles they can hand out).
async fn managed_org(
    state: &AppState,
    user: &CurrentUser,
    slug: &str,
) -> Result<(Org, Perms), Response> {
    let (org, perms) = rbac::org_access(&state.db, user, slug).await?;
    perms.require(Permission::ManageOrgs)?;
    Ok((org, perms))
}

/// Non-admins can't grant or take away permissions they don't hold themselves.
fn within(mine: Perms, role: Perms) -> Result<(), Response> {
    if mine.contains(role) {
        Ok(())
    } else {
        Err(error(
            StatusCode::FORBIDDEN,
            "That role has permissions you don't have here, so you can't assign or change it.",
        ))
    }
}

/// The permissions of a member's current role; 404 if they aren't a member.
async fn current_role(state: &AppState, org: &Org, user_id: i64) -> Result<Perms, Response> {
    rbac::member_perms(&state.db, org.id, user_id)
        .await?
        .ok_or_else(|| not_found("They aren't a member."))
}

async fn members_of(db: &SqlitePool, org: &Org) -> Result<Json<Vec<Member>>, Response> {
    sqlx::query_as(
        "SELECT u.id AS user_id, u.email, u.name, u.picture,
                m.role_id, r.name AS role_name, m.added_at
         FROM org_members m JOIN users u ON u.id = m.user_id JOIN roles r ON r.id = m.role_id
         WHERE m.org_id = ? ORDER BY coalesce(u.name, u.email) COLLATE NOCASE",
    )
    .bind(org.id)
    .fetch_all(db)
    .await
    .map(Json)
    .map_err(db_error)
}

/// GET /api/orgs/{org}/members — needs `manage-orgs` there.
async fn list_members(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<Vec<Member>>, Response> {
    let (org, _) = managed_org(&state, &user, &slug).await?;
    members_of(&state.db, &org).await
}

#[derive(Deserialize)]
struct NewMember {
    user_id: i64,
    role_id: i64,
}

/// POST /api/orgs/{org}/members — add an existing account with a role.
async fn add_member(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Json(body): Json<NewMember>,
) -> Result<(StatusCode, Json<Vec<Member>>), Response> {
    let (org, mine) = managed_org(&state, &user, &slug).await?;
    within(mine, rbac::role_perms(&state.db, body.role_id).await?)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = ?)")
        .bind(body.user_id)
        .fetch_one(&state.db)
        .await
        .map_err(db_error)?;
    if !exists {
        return Err(not_found("That account doesn't exist."));
    }
    let res = sqlx::query(
        "INSERT OR IGNORE INTO org_members (org_id, user_id, role_id) VALUES (?, ?, ?)",
    )
    .bind(org.id)
    .bind(body.user_id)
    .bind(body.role_id)
    .execute(&state.db)
    .await
    .map_err(db_error)?;
    if res.rows_affected() == 0 {
        return Err(error(StatusCode::CONFLICT, "They're already a member."));
    }
    tracing::info!(org = %org.slug, member = body.user_id, role = body.role_id, by = user.id, "member added");
    members_of(&state.db, &org)
        .await
        .map(|m| (StatusCode::CREATED, m))
}

#[derive(Deserialize)]
struct RoleChange {
    role_id: i64,
}

/// PATCH /api/orgs/{org}/members/{user_id} — give a member a different role.
async fn change_role(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, user_id)): UrlPath<(String, i64)>,
    Json(body): Json<RoleChange>,
) -> Result<Json<Vec<Member>>, Response> {
    let (org, mine) = managed_org(&state, &user, &slug).await?;
    within(mine, current_role(&state, &org, user_id).await?)?;
    within(mine, rbac::role_perms(&state.db, body.role_id).await?)?;
    sqlx::query("UPDATE org_members SET role_id = ? WHERE org_id = ? AND user_id = ?")
        .bind(body.role_id)
        .bind(org.id)
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::info!(org = %org.slug, member = user_id, role = body.role_id, by = user.id, "member role changed");
    members_of(&state.db, &org).await
}

/// DELETE /api/orgs/{org}/members/{user_id}
async fn remove_member(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, user_id)): UrlPath<(String, i64)>,
) -> Result<StatusCode, Response> {
    let (org, mine) = managed_org(&state, &user, &slug).await?;
    within(mine, current_role(&state, &org, user_id).await?)?;
    sqlx::query("DELETE FROM org_members WHERE org_id = ? AND user_id = ?")
        .bind(org.id)
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::info!(org = %org.slug, member = user_id, by = user.id, "member removed");
    Ok(StatusCode::NO_CONTENT)
}

/// Everyone who has signed in at least once (rows are created on first sign-in).
#[derive(Serialize, sqlx::FromRow)]
struct UserListItem {
    id: i64,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
    last_login_at: String,
}

/// GET /api/users — all accounts, for the "add member" picker. Needs `manage-orgs` in
/// some org.
async fn list_users(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<UserListItem>>, Response> {
    if !rbac::holds_anywhere(&state.db, &user, Permission::ManageOrgs).await? {
        return Err(rbac::forbidden());
    }
    sqlx::query_as(
        "SELECT id, email, name, picture, last_login_at FROM users
         ORDER BY coalesce(name, email) COLLATE NOCASE",
    )
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db_error)
}

// ---- slugs ----

/// "World Health Organisation" → "world-health-organisation".
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.truncate(MAX_SLUG_LEN);
    slug.trim_end_matches('-').to_owned()
}

/// Lowercase letters, digits and single hyphens; 2–63 chars; not a reserved portal path.
pub fn validate_slug(slug: &str) -> Result<(), &'static str> {
    let ok_chars = slug
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if slug.len() < 2
        || slug.len() > MAX_SLUG_LEN
        || !ok_chars
        || slug.starts_with('-')
        || slug.ends_with('-')
        || slug.contains("--")
    {
        return Err("URL names use 2–63 lowercase letters, digits and single hyphens (e.g. world-health-organisation).");
    }
    if RESERVED_SLUGS.contains(&slug) {
        return Err("That URL name is reserved. Pick another.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_names() {
        assert_eq!(
            slugify("World Health Organisation"),
            "world-health-organisation"
        );
        assert_eq!(slugify("  Acme & Co.  "), "acme-co");
        assert_eq!(slugify("Café Ltd"), "caf-ltd");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn slug_rules() {
        assert!(validate_slug("world-health-organisation").is_ok());
        assert!(validate_slug("a1").is_ok());
        for bad in [
            "",
            "a",
            "-ab",
            "ab-",
            "a--b",
            "Ab",
            "a_b",
            "a/b",
            "..",
            "settings",
            "organisations",
        ] {
            assert!(validate_slug(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(validate_slug(&"a".repeat(MAX_SLUG_LEN + 1)).is_err());
    }
}
