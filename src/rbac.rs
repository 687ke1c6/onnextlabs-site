//! Org-scoped access control.
//!
//! - Permissions are a fixed list defined here (`Permission`). Add a variant (plus its
//!   entry in `Permission::ALL`) when a feature needs one; the UI picks it up from the API.
//! - Roles are user-defined named sets of permissions, shared by every org (`roles`,
//!   `role_permissions`).
//! - Each org membership has exactly one role (`org_members.role_id`), so someone can be an
//!   Admin-org in one org and a Member in another.
//! - Site admins hold every permission in every org. That comes only from ADMIN_EMAILS or
//!   the `admin` user role (`grant-role` CLI), never from an org role.
//!
//! Guard rule for delegated management: a non-admin can only grant, change or take away a
//! role whose permissions they hold themselves in that org.

use std::sync::Arc;

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::Response,
    routing::{get, put},
    Json, Router,
};
use serde::{ser::SerializeSeq, Deserialize, Serialize, Serializer};
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::{
    api::{bad_request, db_error, error, not_found},
    auth::CurrentUser,
    orgs::{self, Org},
    AppState,
};

const MAX_ROLE_NAME_CHARS: usize = 50;
const MAX_ROLE_DESCRIPTION_CHARS: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Permission {
    ReadDocs,
    WriteDocs,
    ManageOrgs,
    WriteRoles,
}

impl Permission {
    /// Every permission, in display order.
    pub const ALL: [Permission; 4] = [
        Permission::ReadDocs,
        Permission::WriteDocs,
        Permission::ManageOrgs,
        Permission::WriteRoles,
    ];

    /// Stored and sent over the API.
    pub fn key(self) -> &'static str {
        match self {
            Permission::ReadDocs => "read-docs",
            Permission::WriteDocs => "write-docs",
            Permission::ManageOrgs => "manage-orgs",
            Permission::WriteRoles => "write-roles",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    fn description(self) -> &'static str {
        match self {
            Permission::ReadDocs => {
                "Browse the organisation's documents and download private ones."
            }
            Permission::WriteDocs => "Upload documents and create folders.",
            Permission::ManageOrgs => "Add and remove members and change their roles.",
            Permission::WriteRoles => {
                "Create, edit and delete roles. Roles are shared by every organisation."
            }
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// A set of permissions. Serialises as a list of keys.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Perms(u8);

impl Perms {
    pub fn all() -> Self {
        let mut perms = Perms::default();
        for p in Permission::ALL {
            perms.insert(p);
        }
        perms
    }

    pub fn has(self, p: Permission) -> bool {
        self.0 & p.bit() != 0
    }

    pub fn contains(self, other: Perms) -> bool {
        self.0 & other.0 == other.0
    }

    fn insert(&mut self, p: Permission) {
        self.0 |= p.bit();
    }

    fn iter(self) -> impl Iterator<Item = Permission> {
        Permission::ALL.into_iter().filter(move |p| self.has(*p))
    }

    /// From a comma-separated list (`group_concat`); unknown keys are ignored.
    pub fn from_csv(csv: Option<&str>) -> Self {
        let mut perms = Perms::default();
        for p in csv.unwrap_or("").split(',').filter_map(Permission::parse) {
            perms.insert(p);
        }
        perms
    }

    /// 403 unless `p` is in the set.
    pub fn require(self, p: Permission) -> Result<(), Response> {
        if self.has(p) {
            Ok(())
        } else {
            Err(forbidden())
        }
    }
}

impl Serialize for Perms {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        for p in self.iter() {
            seq.serialize_element(p.key())?;
        }
        seq.end()
    }
}

pub fn forbidden() -> Response {
    error(
        StatusCode::FORBIDDEN,
        "You don't have permission to do that.",
    )
}

/// The org and the user's permissions in it. Orgs they can't see (unknown, or not a
/// member) are reported as not found, so their existence isn't revealed.
pub async fn org_access(
    db: &SqlitePool,
    user: &CurrentUser,
    slug: &str,
) -> Result<(Org, Perms), Response> {
    let org = orgs::find(db, slug).await?;
    if user.is_admin() {
        return Ok((org, Perms::all()));
    }
    match member_perms(db, org.id, user.id).await? {
        Some(perms) => Ok((org, perms)),
        None => Err(not_found("Organisation not found.")),
    }
}

/// The org, if the user holds `p` in it (404 if they can't see it, 403 if they lack `p`).
pub async fn authorize(
    db: &SqlitePool,
    user: &CurrentUser,
    slug: &str,
    p: Permission,
) -> Result<Org, Response> {
    let (org, perms) = org_access(db, user, slug).await?;
    perms.require(p)?;
    Ok(org)
}

/// The permissions of `user_id`'s role in the org, or None if they aren't a member.
pub async fn member_perms(
    db: &SqlitePool,
    org_id: i64,
    user_id: i64,
) -> Result<Option<Perms>, Response> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT (SELECT group_concat(p.permission) FROM role_permissions p WHERE p.role_id = m.role_id)
         FROM org_members m WHERE m.org_id = ? AND m.user_id = ?",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_optional(db)
    .await
    .map_err(db_error)?;
    Ok(row.map(|(csv,)| Perms::from_csv(csv.as_deref())))
}

/// Whether the user holds `p` in at least one org (always true for admins).
pub async fn holds_anywhere(
    db: &SqlitePool,
    user: &CurrentUser,
    p: Permission,
) -> Result<bool, Response> {
    if user.is_admin() {
        return Ok(true);
    }
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM org_members m JOIN role_permissions p ON p.role_id = m.role_id
             WHERE m.user_id = ? AND p.permission = ?)",
    )
    .bind(user.id)
    .bind(p.key())
    .fetch_one(db)
    .await
    .map_err(db_error)
}

/// A role's permissions; 400 if the role doesn't exist.
pub async fn role_perms(db: &SqlitePool, role_id: i64) -> Result<Perms, Response> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT (SELECT group_concat(permission) FROM role_permissions WHERE role_id = r.id)
         FROM roles r WHERE r.id = ?",
    )
    .bind(role_id)
    .fetch_optional(db)
    .await
    .map_err(db_error)?;
    row.map(|(csv,)| Perms::from_csv(csv.as_deref()))
        .ok_or_else(|| bad_request("That role doesn't exist."))
}

// ---- roles API ----

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/roles", get(list_roles).post(create_role))
        .route("/api/roles/{id}", put(update_role).delete(delete_role))
}

#[derive(Serialize, sqlx::FromRow)]
struct RoleRow {
    id: i64,
    name: String,
    description: String,
    #[serde(skip)]
    permission_csv: Option<String>,
    #[sqlx(skip)]
    permissions: Perms,
    member_count: i64,
}

macro_rules! select_role {
    ($tail:literal) => {
        concat!(
            "SELECT r.id, r.name, r.description,
                    (SELECT group_concat(permission) FROM role_permissions WHERE role_id = r.id)
                        AS permission_csv,
                    (SELECT count(*) FROM org_members WHERE role_id = r.id) AS member_count
             FROM roles r ",
            $tail
        )
    };
}

fn with_perms(mut role: RoleRow) -> RoleRow {
    role.permissions = Perms::from_csv(role.permission_csv.as_deref());
    role
}

/// GET /api/roles — every role, plus the permissions roles can be given. Any signed-in
/// user: org managers need it to assign roles.
async fn list_roles(
    _user: CurrentUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, Response> {
    let roles: Vec<RoleRow> = sqlx::query_as(select_role!("ORDER BY r.name COLLATE NOCASE"))
        .fetch_all(&state.db)
        .await
        .map_err(db_error)?;
    let roles: Vec<RoleRow> = roles.into_iter().map(with_perms).collect();
    let permissions: Vec<Value> = Permission::ALL
        .iter()
        .map(|p| json!({ "key": p.key(), "description": p.description() }))
        .collect();
    Ok(Json(json!({ "permissions": permissions, "roles": roles })))
}

#[derive(Deserialize)]
struct RoleInput {
    name: String,
    #[serde(default)]
    description: String,
    permissions: Vec<String>,
}

struct ValidRole {
    name: String,
    description: String,
    perms: Perms,
}

fn validate_role(input: RoleInput) -> Result<ValidRole, Response> {
    let name = input.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > MAX_ROLE_NAME_CHARS {
        return Err(bad_request("Give the role a name of up to 50 characters."));
    }
    let description = input.description.trim().to_owned();
    if description.chars().count() > MAX_ROLE_DESCRIPTION_CHARS {
        return Err(bad_request("Keep the description to 200 characters."));
    }
    let mut perms = Perms::default();
    for key in &input.permissions {
        let p = Permission::parse(key).ok_or_else(|| bad_request("Unknown permission."))?;
        perms.insert(p);
    }
    Ok(ValidRole {
        name,
        description,
        perms,
    })
}

async fn require_write_roles(db: &SqlitePool, user: &CurrentUser) -> Result<(), Response> {
    if holds_anywhere(db, user, Permission::WriteRoles).await? {
        Ok(())
    } else {
        Err(forbidden())
    }
}

fn name_taken(e: sqlx::Error) -> Response {
    match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => error(
            StatusCode::CONFLICT,
            "A role with that name already exists.",
        ),
        e => db_error(e),
    }
}

/// Replaces the role's permissions and returns the stored role.
async fn save_role(db: &SqlitePool, id: Option<i64>, role: ValidRole) -> Result<RoleRow, Response> {
    let mut tx = db.begin().await.map_err(db_error)?;
    let id: i64 = match id {
        None => {
            sqlx::query_scalar("INSERT INTO roles (name, description) VALUES (?, ?) RETURNING id")
                .bind(&role.name)
                .bind(&role.description)
                .fetch_one(&mut *tx)
                .await
                .map_err(name_taken)?
        }
        Some(id) => {
            let res = sqlx::query("UPDATE roles SET name = ?, description = ? WHERE id = ?")
                .bind(&role.name)
                .bind(&role.description)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(name_taken)?;
            if res.rows_affected() == 0 {
                return Err(not_found("Role not found."));
            }
            sqlx::query("DELETE FROM role_permissions WHERE role_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            id
        }
    };
    for p in role.perms.iter() {
        sqlx::query("INSERT INTO role_permissions (role_id, permission) VALUES (?, ?)")
            .bind(id)
            .bind(p.key())
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    sqlx::query_as(select_role!("WHERE r.id = ?"))
        .bind(id)
        .fetch_one(db)
        .await
        .map(with_perms)
        .map_err(db_error)
}

/// POST /api/roles — needs `write-roles` in some org.
async fn create_role(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    Json(input): Json<RoleInput>,
) -> Result<(StatusCode, Json<RoleRow>), Response> {
    require_write_roles(&state.db, &user).await?;
    let role = save_role(&state.db, None, validate_role(input)?).await?;
    tracing::info!(role = %role.name, user = user.id, "role created");
    Ok((StatusCode::CREATED, Json(role)))
}

/// PUT /api/roles/{id} — rename and/or replace the permissions.
async fn update_role(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<i64>,
    Json(input): Json<RoleInput>,
) -> Result<Json<RoleRow>, Response> {
    require_write_roles(&state.db, &user).await?;
    let role = save_role(&state.db, Some(id), validate_role(input)?).await?;
    tracing::info!(role = %role.name, user = user.id, "role updated");
    Ok(Json(role))
}

/// DELETE /api/roles/{id} — refused while anyone holds the role.
async fn delete_role(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<i64>,
) -> Result<StatusCode, Response> {
    require_write_roles(&state.db, &user).await?;
    let in_use = || {
        error(
            StatusCode::CONFLICT,
            "People still have this role. Give them another role first.",
        )
    };
    // Checked up front for a clear message; the ON DELETE RESTRICT foreign key still
    // guards the race (SQLite reports it as a constraint error, code 1811).
    let holders: i64 = sqlx::query_scalar("SELECT count(*) FROM org_members WHERE role_id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(db_error)?;
    if holders > 0 {
        return Err(in_use());
    }
    let res = sqlx::query("DELETE FROM roles WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await;
    match res {
        Ok(r) if r.rows_affected() == 0 => Err(not_found("Role not found.")),
        Ok(_) => {
            tracing::info!(role = id, user = user.id, "role deleted");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(sqlx::Error::Database(d))
            if d.kind() == sqlx::error::ErrorKind::ForeignKeyViolation
                || d.code().as_deref() == Some("1811") =>
        {
            Err(in_use())
        }
        Err(e) => Err(db_error(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.key()), Some(p));
        }
        assert_eq!(Permission::parse("root"), None);
    }

    #[test]
    fn perms_from_csv_and_subsets() {
        let member = Perms::from_csv(Some("read-docs"));
        let editor = Perms::from_csv(Some("write-docs,read-docs,bogus"));
        assert!(member.has(Permission::ReadDocs) && !member.has(Permission::WriteDocs));
        assert!(editor.contains(member));
        assert!(!member.contains(editor));
        assert!(Perms::all().contains(editor));
        assert_eq!(Perms::from_csv(None), Perms::default());
        assert!(Perms::default().require(Permission::ReadDocs).is_err());
        assert_eq!(
            serde_json::to_value(editor).unwrap(),
            json!(["read-docs", "write-docs"])
        );
    }
}
