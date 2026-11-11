//! Account administration from the command line (`onnextlabs-site grant-role ...`), for
//! granting access before there's an admin UI to do it.

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

pub async fn grant_role(db: &SqlitePool, email: &str, role: &str) -> Result<()> {
    let id = user_id_by_email(db, email).await?;
    let res = sqlx::query("INSERT OR IGNORE INTO user_roles (user_id, role) VALUES (?, ?)")
        .bind(id)
        .bind(role)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        println!("{email} already has role {role}");
    } else {
        println!("granted {role} to {email}");
    }
    Ok(())
}

pub async fn revoke_role(db: &SqlitePool, email: &str, role: &str) -> Result<()> {
    let id = user_id_by_email(db, email).await?;
    let res = sqlx::query("DELETE FROM user_roles WHERE user_id = ? AND role = ?")
        .bind(id)
        .bind(role)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        println!("{email} did not have role {role}");
    } else {
        println!("revoked {role} from {email}");
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i64,
    email: Option<String>,
    name: Option<String>,
    last_login_at: String,
    roles: Option<String>,
}

pub async fn list_users(db: &SqlitePool) -> Result<()> {
    let rows: Vec<UserRow> = sqlx::query_as(
        "SELECT u.id, u.email, u.name, u.last_login_at, group_concat(r.role, ',') AS roles
         FROM users u LEFT JOIN user_roles r ON r.user_id = u.id
         GROUP BY u.id ORDER BY u.id",
    )
    .fetch_all(db)
    .await?;
    if rows.is_empty() {
        println!("no users yet");
    }
    for u in rows {
        println!(
            "{}\t{}\t{}\tlast login {}\troles: {}",
            u.id,
            u.email.as_deref().unwrap_or("-"),
            u.name.as_deref().unwrap_or("-"),
            u.last_login_at,
            u.roles.as_deref().unwrap_or("none"),
        );
    }
    Ok(())
}
