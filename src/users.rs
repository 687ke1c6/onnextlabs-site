//! Account administration from the command line (`onnextlabs-site grant-admin ...`): for
//! bootstrapping and recovery; the portal does the same day to day.

use sqlx::SqlitePool;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The single user with this email (they must have signed in at least once).
async fn user_id_by_email(db: &SqlitePool, email: &str) -> Result<i64> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email.trim())
        .fetch_all(db)
        .await?;
    match ids.as_slice() {
        [id] => Ok(*id),
        [] => Err(format!("no user with email {email}; they need to sign in once first").into()),
        _ => {
            Err(format!("several users share email {email}; resolve by id in the database").into())
        }
    }
}

/// Sets or clears the account's admin flag (the portal's Manage → Users does the same).
pub async fn set_admin(db: &SqlitePool, email: &str, admin: bool) -> Result<()> {
    let id = user_id_by_email(db, email).await?;
    sqlx::query("UPDATE users SET is_admin = ? WHERE id = ?")
        .bind(admin)
        .bind(id)
        .execute(db)
        .await?;
    println!(
        "{email} is {}an admin",
        if admin { "now " } else { "no longer " }
    );
    Ok(())
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i64,
    email: Option<String>,
    name: Option<String>,
    last_login_at: String,
    is_admin: bool,
}

pub async fn list_users(db: &SqlitePool) -> Result<()> {
    let rows: Vec<UserRow> =
        sqlx::query_as("SELECT id, email, name, last_login_at, is_admin FROM users ORDER BY id")
            .fetch_all(db)
            .await?;
    if rows.is_empty() {
        println!("no users yet");
    }
    for u in rows {
        println!(
            "{}\t{}\t{}\tlast login {}{}",
            u.id,
            u.email.as_deref().unwrap_or("-"),
            u.name.as_deref().unwrap_or("-"),
            u.last_login_at,
            if u.is_admin { "\tadmin" } else { "" },
        );
    }
    Ok(())
}

// ---- organisations ----

/// Creates an org. The slug (URL and folder name) is derived from the name unless given.
pub async fn create_org(db: &SqlitePool, name: &str, slug: Option<&str>) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err("organisation name can't be empty".into());
    }
    let slug = slug
        .map(str::to_owned)
        .unwrap_or_else(|| crate::orgs::slugify(name));
    crate::orgs::validate_slug(&slug)?;
    sqlx::query("INSERT INTO organisations (slug, name) VALUES (?, ?)")
        .bind(&slug)
        .bind(name)
        .execute(db)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                format!("an organisation already uses the slug {slug}").into()
            }
            e => Box::<dyn std::error::Error>::from(e),
        })?;
    println!("created {name} at /portal/{slug}");
    Ok(())
}

async fn org_id_by_slug(db: &SqlitePool, slug: &str) -> Result<i64> {
    sqlx::query_scalar("SELECT id FROM organisations WHERE slug = ?")
        .bind(slug)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| format!("no organisation with slug {slug}").into())
}

/// Adds a user to an org with the named role, or changes their role if already a member.
pub async fn add_member(db: &SqlitePool, slug: &str, email: &str, role: &str) -> Result<()> {
    let org = org_id_by_slug(db, slug).await?;
    let user = user_id_by_email(db, email).await?;
    let role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(role)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| format!("no role named {role}"))?;
    sqlx::query(
        "INSERT INTO org_members (org_id, user_id, role_id) VALUES (?, ?, ?)
         ON CONFLICT (org_id, user_id) DO UPDATE SET role_id = excluded.role_id",
    )
    .bind(org)
    .bind(user)
    .bind(role_id)
    .execute(db)
    .await?;
    println!("{email} is now {role} in {slug}");
    Ok(())
}

pub async fn remove_member(db: &SqlitePool, slug: &str, email: &str) -> Result<()> {
    let org = org_id_by_slug(db, slug).await?;
    let user = user_id_by_email(db, email).await?;
    let res = sqlx::query("DELETE FROM org_members WHERE org_id = ? AND user_id = ?")
        .bind(org)
        .bind(user)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        println!("{email} wasn't a member of {slug}");
    } else {
        println!("removed {email} from {slug}");
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct OrgRow {
    slug: String,
    name: String,
    members: Option<String>,
}

pub async fn list_orgs(db: &SqlitePool) -> Result<()> {
    let rows: Vec<OrgRow> = sqlx::query_as(
        "SELECT o.slug, o.name, group_concat(u.email || ' (' || r.name || ')', ', ') AS members
         FROM organisations o
         LEFT JOIN org_members m ON m.org_id = o.id
         LEFT JOIN users u ON u.id = m.user_id
         LEFT JOIN roles r ON r.id = m.role_id
         GROUP BY o.id ORDER BY o.name COLLATE NOCASE",
    )
    .fetch_all(db)
    .await?;
    if rows.is_empty() {
        println!("no organisations yet");
    }
    for o in rows {
        println!(
            "{}\t{}\tmembers: {}",
            o.slug,
            o.name,
            o.members.as_deref().unwrap_or("none")
        );
    }
    Ok(())
}
