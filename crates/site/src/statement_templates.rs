//! Statement templates: how statements print as PDF. They belong to the org, and any account
//! prints with any of them; the print dialog picks one (the account remembers the last).
//!
//! A template picks a built-in layout (`statements_pdf::LAYOUTS`) and sets its text (title,
//! subtitle, header fields, footer: all may use `{name}` variables, checked here when saved), a
//! logo (an image in Documents), a letterhead (a PDF in Documents drawn behind the pages, on every
//! page or the first only), margins, paper, which columns and sections print, and how dates and
//! money look. Every org has at least one: listing them makes "Standard" if there are none.
//!
//! API: list and get need `read-statements`; create, save and delete `write-statements`.

use crate::db::{self, Db};
use std::sync::Arc;

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::Response,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use statements_pdf::{vars, Column, ColumnKey, DateStyle, Field, TotalsOptions};

use crate::{
    api::{bad_request, db_error, error, not_found},
    auth::CurrentUser,
    documents,
    orgs::Org,
    rbac::{self, Permission},
    AppState,
};

pub const MAX_LOGO_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_LETTERHEAD_BYTES: usize = 10 * 1024 * 1024;
const LOGO_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "svg", "webp"];
/// Top and bottom margins, in mm.
const MARGINS_MM: std::ops::RangeInclusive<u32> = 5..=150;
const MAX_FIELDS: usize = 20;
const MAX_COLUMN_LABEL: usize = 40;
const STANDARD: &str = "Standard";

/// The columns a [`Row`] is read from.
const COLUMNS: &str = "id, name, layout, title, subtitle, header_fields, footer, logo_document_id,
                       letterhead_document_id, letterhead_every_page, margin_top_mm,
                       margin_bottom_mm, paper, columns, totals, show_summary, group_by_type,
                       date_style, currency_symbol, updated_at";

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/orgs/{org}/statement-templates",
            get(list).post(create),
        )
        .route(
            "/api/orgs/{org}/statement-templates/{id}",
            get(get_one).put(save).delete(remove),
        )
}

/// What a template sets (everything but its id).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Settings {
    pub name: String,
    /// `standard` or `bank`.
    pub layout: String,
    pub title: String,
    pub subtitle: String,
    pub header_fields: Vec<Field>,
    pub footer: String,
    pub logo_document_id: Option<i64>,
    pub letterhead_document_id: Option<i64>,
    pub letterhead_every_page: bool,
    pub margin_top_mm: u32,
    pub margin_bottom_mm: u32,
    /// `a4` or `us-letter`.
    pub paper: String,
    /// The transaction table's columns, in order, with their headings.
    pub columns: Vec<Column>,
    /// Which totals print, and their labels.
    pub totals: TotalsOptions,
    /// The totals by type table (standard layout).
    pub show_summary: bool,
    pub group_by_type: bool,
    pub date_style: DateStyle,
    pub currency_symbol: String,
}

impl Settings {
    /// A new template's settings for `layout`.
    pub fn new(name: &str, layout: &str) -> Settings {
        let bank = layout == "bank";
        Settings {
            name: name.into(),
            layout: if bank { "bank" } else { "standard" }.into(),
            title: "Statement".into(),
            subtitle: if bank { "{account.name}" } else { "" }.into(),
            header_fields: statements_pdf::default_fields(layout),
            footer: String::new(),
            logo_document_id: None,
            letterhead_document_id: None,
            letterhead_every_page: true,
            margin_top_mm: 18,
            margin_bottom_mm: 22,
            paper: "a4".into(),
            columns: statements_pdf::default_columns(layout),
            totals: statements_pdf::default_totals(layout),
            show_summary: !bank,
            group_by_type: false,
            date_style: if bank {
                DateStyle::Padded
            } else {
                DateStyle::Short
            },
            currency_symbol: if bank { "R" } else { "" }.into(),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Template {
    pub id: i64,
    #[serde(flatten)]
    pub settings: Settings,
    pub updated_at: String,
}

/// A template as stored (header fields as JSON).
#[derive(Deserialize)]
struct Row {
    id: i64,
    name: String,
    layout: String,
    title: String,
    subtitle: String,
    header_fields: String,
    footer: String,
    logo_document_id: Option<i64>,
    letterhead_document_id: Option<i64>,
    letterhead_every_page: bool,
    margin_top_mm: u32,
    margin_bottom_mm: u32,
    paper: String,
    columns: String,
    totals: String,
    show_summary: bool,
    group_by_type: bool,
    date_style: DateStyle,
    currency_symbol: String,
    updated_at: String,
}

impl From<Row> for Template {
    fn from(r: Row) -> Template {
        Template {
            id: r.id,
            settings: Settings {
                name: r.name,
                layout: r.layout,
                title: r.title,
                subtitle: r.subtitle,
                header_fields: serde_json::from_str(&r.header_fields).unwrap_or_default(),
                footer: r.footer,
                logo_document_id: r.logo_document_id,
                letterhead_document_id: r.letterhead_document_id,
                letterhead_every_page: r.letterhead_every_page,
                margin_top_mm: r.margin_top_mm,
                margin_bottom_mm: r.margin_bottom_mm,
                paper: r.paper,
                columns: serde_json::from_str(&r.columns).unwrap_or_default(),
                totals: serde_json::from_str(&r.totals).unwrap_or_default(),
                show_summary: r.show_summary,
                group_by_type: r.group_by_type,
                date_style: r.date_style,
                currency_symbol: r.currency_symbol,
            },
            updated_at: r.updated_at,
        }
    }
}

/// The template, if it's the org's.
pub async fn find(db: &Db, org_id: i64, id: i64) -> db::Result<Option<Template>> {
    let row: Option<Row> = db::query_as(db::Trusted(format!(
        "SELECT {COLUMNS} FROM statement_templates WHERE id = ? AND org_id = ?"
    )))
    .bind(id)
    .bind(org_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(Template::from))
}

/// The org's templates by name, making "Standard" first if it has none.
pub async fn all(db: &Db, org_id: i64) -> db::Result<Vec<Template>> {
    let read = || async {
        let rows: Vec<Row> = db::query_as(db::Trusted(format!(
            "SELECT {COLUMNS} FROM statement_templates WHERE org_id = ? ORDER BY name"
        )))
        .bind(org_id)
        .fetch_all(db)
        .await?;
        db::Result::Ok(rows.into_iter().map(Template::from).collect::<Vec<_>>())
    };
    let list = read().await?;
    if !list.is_empty() {
        return Ok(list);
    }
    // Two first visits at once both try; the name is unique, so one wins.
    insert(db, org_id, &Settings::new(STANDARD, "standard"), true).await?;
    read().await
}

/// The template to print with: `wanted` if it's the org's, else the account's last one, else the
/// org's first.
pub async fn for_printing(
    db: &Db,
    org_id: i64,
    wanted: Option<i64>,
    last: Option<i64>,
) -> Result<Template, Response> {
    if let Some(id) = wanted {
        return find(db, org_id, id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| not_found("That template isn't in this organisation."));
    }
    if let Some(id) = last {
        if let Some(t) = find(db, org_id, id).await.map_err(db_error)? {
            return Ok(t);
        }
    }
    all(db, org_id)
        .await
        .map_err(db_error)?
        .into_iter()
        .next()
        .ok_or_else(|| error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong."))
}

async fn insert(
    db: &Db,
    org_id: i64,
    s: &Settings,
    ignore_existing: bool,
) -> db::Result<Option<i64>> {
    let sql = if ignore_existing {
        "INSERT OR IGNORE INTO statement_templates"
    } else {
        "INSERT INTO statement_templates"
    };
    db::query_scalar(db::Trusted(format!(
        "{sql}
             (org_id, name, layout, title, subtitle, header_fields, footer, logo_document_id,
              letterhead_document_id, letterhead_every_page, margin_top_mm, margin_bottom_mm,
              paper, columns, totals, show_summary, group_by_type, date_style, currency_symbol)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id"
    )))
    .bind(org_id)
    .bind(&s.name)
    .bind(&s.layout)
    .bind(&s.title)
    .bind(&s.subtitle)
    .bind(serde_json::to_string(&s.header_fields).expect("fields serialise"))
    .bind(&s.footer)
    .bind(s.logo_document_id)
    .bind(s.letterhead_document_id)
    .bind(s.letterhead_every_page)
    .bind(s.margin_top_mm)
    .bind(s.margin_bottom_mm)
    .bind(&s.paper)
    .bind(serde_json::to_string(&s.columns).expect("columns serialise"))
    .bind(serde_json::to_string(&s.totals).expect("totals serialise"))
    .bind(s.show_summary)
    .bind(s.group_by_type)
    .bind(date_style(s.date_style))
    .bind(&s.currency_symbol)
    .fetch_optional(db)
    .await
}

fn date_style(d: DateStyle) -> &'static str {
    match d {
        DateStyle::Short => "short",
        DateStyle::Padded => "padded",
    }
}

#[derive(Serialize)]
pub struct DocumentPlace {
    folder: String,
    filename: String,
}

/// A template with where its logo and letterhead are in Documents, for the editor to show.
#[derive(Serialize)]
struct TemplateView {
    #[serde(flatten)]
    template: Template,
    logo: Option<DocumentPlace>,
    letterhead: Option<DocumentPlace>,
}

async fn view(db: &Db, org_id: i64, template: Template) -> Result<TemplateView, Response> {
    let place = async |id: Option<i64>| -> Result<Option<DocumentPlace>, Response> {
        Ok(match id {
            Some(id) => documents::find_in_org(db, org_id, id)
                .await?
                .map(|(folder, filename)| DocumentPlace { folder, filename }),
            None => None,
        })
    };
    let logo = place(template.settings.logo_document_id).await?;
    let letterhead = place(template.settings.letterhead_document_id).await?;
    Ok(TemplateView {
        template,
        logo,
        letterhead,
    })
}

#[derive(Serialize)]
struct Choice {
    name: &'static str,
    label: &'static str,
}

#[derive(Serialize)]
struct ColumnChoice {
    key: &'static str,
    /// Its usual heading.
    label: &'static str,
    /// What it shows.
    hint: &'static str,
}

#[derive(Serialize)]
struct TemplateList {
    templates: Vec<Template>,
    /// The `{name}` variables text can use, with what each prints.
    variables: Vec<Choice>,
    layouts: Vec<Choice>,
    /// The columns a template's table can have.
    columns: Vec<ColumnChoice>,
    /// The figures a template's summary can have.
    summary_figures: Vec<ColumnChoice>,
}

/// GET /api/orgs/{org}/statement-templates
async fn list(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
) -> Result<Json<TemplateList>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    let choices = |list: &[(&'static str, &'static str)]| {
        list.iter()
            .map(|&(name, label)| Choice { name, label })
            .collect()
    };
    Ok(Json(TemplateList {
        templates: all(&state.db, org.id).await.map_err(db_error)?,
        variables: choices(&vars::VARIABLES),
        layouts: choices(&statements_pdf::LAYOUTS),
        columns: statements_pdf::COLUMN_KEYS
            .iter()
            .map(|&(key, label, hint)| ColumnChoice {
                key: key.name(),
                label,
                hint,
            })
            .collect(),
        summary_figures: statements_pdf::SUMMARY_KEYS
            .iter()
            .map(|&(key, label, hint)| ColumnChoice {
                key: key.name(),
                label,
                hint,
            })
            .collect(),
    }))
}

/// GET /api/orgs/{org}/statement-templates/{id}
async fn get_one(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<Json<TemplateView>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::ReadStatements).await?;
    let t = find(&state.db, org.id, id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Template not found."))?;
    Ok(Json(view(&state.db, org.id, t).await?))
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
    #[serde(default)]
    layout: Option<String>,
    /// Start from a copy of this template rather than the layout's defaults.
    #[serde(default)]
    copy_of: Option<i64>,
}

/// POST /api/orgs/{org}/statement-templates — a new template, from a layout's defaults or a
/// copy of another. Returns it.
async fn create(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath(slug): UrlPath<String>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<TemplateView>), Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let mut s = match body.copy_of {
        Some(id) => {
            find(&state.db, org.id, id)
                .await
                .map_err(db_error)?
                .ok_or_else(|| not_found("Template not found."))?
                .settings
        }
        None => Settings::new("", body.layout.as_deref().unwrap_or("standard")),
    };
    s.name = body.name;
    let s = check(&state.db, org.id, s).await?;
    let id = insert(&state.db, org.id, &s, false)
        .await
        .map_err(|e| conflict(&e).unwrap_or_else(|| db_error(e)))?
        .ok_or_else(|| error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong."))?;
    tracing::info!(org = %org.slug, template = id, name = %s.name, user = user.id, "statement template created");
    let t = find(&state.db, org.id, id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Template not found."))?;
    Ok((StatusCode::CREATED, Json(view(&state.db, org.id, t).await?)))
}

/// PUT /api/orgs/{org}/statement-templates/{id}
async fn save(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
    Json(body): Json<Settings>,
) -> Result<Json<TemplateView>, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    find(&state.db, org.id, id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Template not found."))?;
    let s = check(&state.db, org.id, body).await?;
    if let Some(doc) = s.letterhead_document_id {
        // Read it now, so a PDF that can't be used is turned away here rather than at printing.
        letterhead(&state, &org, doc).await?;
    }
    db::query(
        "UPDATE statement_templates SET
             name = ?, layout = ?, title = ?, subtitle = ?, header_fields = ?, footer = ?,
             logo_document_id = ?, letterhead_document_id = ?, letterhead_every_page = ?,
             margin_top_mm = ?, margin_bottom_mm = ?, paper = ?, columns = ?, totals = ?,
             show_summary = ?, group_by_type = ?,
             date_style = ?, currency_symbol = ?,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(&s.name)
    .bind(&s.layout)
    .bind(&s.title)
    .bind(&s.subtitle)
    .bind(serde_json::to_string(&s.header_fields).expect("fields serialise"))
    .bind(&s.footer)
    .bind(s.logo_document_id)
    .bind(s.letterhead_document_id)
    .bind(s.letterhead_every_page)
    .bind(s.margin_top_mm)
    .bind(s.margin_bottom_mm)
    .bind(&s.paper)
    .bind(serde_json::to_string(&s.columns).expect("columns serialise"))
    .bind(serde_json::to_string(&s.totals).expect("totals serialise"))
    .bind(s.show_summary)
    .bind(s.group_by_type)
    .bind(date_style(s.date_style))
    .bind(&s.currency_symbol)
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(|e| conflict(&e).unwrap_or_else(|| db_error(e)))?;
    tracing::info!(org = %org.slug, template = id, user = user.id, "statement template saved");
    let t = find(&state.db, org.id, id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Template not found."))?;
    Ok(Json(view(&state.db, org.id, t).await?))
}

/// DELETE /api/orgs/{org}/statement-templates/{id} — accounts that last used it forget it.
async fn remove(
    user: CurrentUser,
    State(state): State<Arc<AppState>>,
    UrlPath((slug, id)): UrlPath<(String, i64)>,
) -> Result<StatusCode, Response> {
    let org = rbac::authorize(&state.db, &user, &slug, Permission::WriteStatements).await?;
    let t = find(&state.db, org.id, id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found("Template not found."))?;
    db::query("DELETE FROM statement_templates WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    tracing::warn!(org = %org.slug, template = id, name = %t.settings.name, user = user.id, "statement template deleted");
    Ok(StatusCode::NO_CONTENT)
}

fn conflict(e: &db::Error) -> Option<Response> {
    e.is_unique_violation().then(|| {
        error(
            StatusCode::CONFLICT,
            "There's already a template with that name.",
        )
    })
}

/// Trimmed and within limits; text whose variables all exist; a logo that's an image and a
/// letterhead that's a PDF, both in the org's Documents. (Whether the letterhead can be read is
/// checked by [`letterhead`].)
pub async fn check(db: &Db, org_id: i64, s: Settings) -> Result<Settings, Response> {
    let s = check_text(s)?;
    if let Some(id) = s.logo_document_id {
        let (_, filename) = documents::find_in_org(db, org_id, id)
            .await?
            .ok_or_else(|| bad_request("That logo isn't in Documents."))?;
        logo_extension(&filename).ok_or_else(|| {
            bad_request("The logo must be an image: PNG, JPEG, GIF, SVG or WebP.")
        })?;
    }
    if let Some(id) = s.letterhead_document_id {
        let (_, filename) = documents::find_in_org(db, org_id, id)
            .await?
            .ok_or_else(|| bad_request("That letterhead isn't in Documents."))?;
        if !is_pdf(&filename) {
            return Err(bad_request("The letterhead must be a PDF."));
        }
    }
    Ok(s)
}

/// The checks that don't need the database.
fn check_text(mut s: Settings) -> Result<Settings, Response> {
    let text = |value: &str, what: &str, max: usize| -> Result<String, Response> {
        let v = value.trim();
        if v.chars().count() > max || v.chars().any(char::is_control) {
            return Err(bad_request(&format!(
                "The {what} can be at most {max} characters, on one line."
            )));
        }
        vars::check(v).map_err(|e| bad_request(&format!("In the {what}: {e}.")))?;
        Ok(v.to_string())
    };
    s.name = text(&s.name, "name", 100)?;
    if s.name.is_empty() {
        return Err(bad_request("Give the template a name."));
    }
    if !statements_pdf::LAYOUTS.iter().any(|(l, _)| *l == s.layout) {
        return Err(bad_request("Pick a layout."));
    }
    // May be blank: a letterhead may already say what the document is.
    s.title = text(&s.title, "title", 120)?;
    s.subtitle = text(&s.subtitle, "subtitle", 200)?;
    s.footer = text(&s.footer, "footer", 300)?;
    let mut fields = Vec::with_capacity(s.header_fields.len());
    for (i, f) in s.header_fields.iter().enumerate() {
        let n = i + 1;
        let label = text(&f.label, &format!("label of header field {n}"), 60)?;
        let value = text(&f.value, &format!("value of header field {n}"), 200)?;
        if !label.is_empty() || !value.is_empty() {
            fields.push(Field { label, value });
        }
    }
    if fields.len() > MAX_FIELDS {
        return Err(bad_request(&format!(
            "A template can have at most {MAX_FIELDS} header fields."
        )));
    }
    s.header_fields = fields;
    let mut seen = std::collections::HashSet::new();
    for c in &mut s.columns {
        if !seen.insert(c.key) {
            return Err(bad_request(&format!(
                "The {} column is in the table twice.",
                c.key.label()
            )));
        }
        let label = c.label.trim();
        if label.chars().count() > MAX_COLUMN_LABEL || label.chars().any(char::is_control) {
            return Err(bad_request(&format!(
                "Column headings can be at most {MAX_COLUMN_LABEL} characters, on one line."
            )));
        }
        c.label = if label.is_empty() {
            c.key.label()
        } else {
            label
        }
        .to_string();
    }
    let t = &mut s.totals;
    let mut seen = std::collections::HashSet::new();
    for item in &mut t.summary {
        if !seen.insert(item.key) {
            return Err(bad_request(&format!(
                "{} is in the summary twice.",
                item.key.label()
            )));
        }
        let label = text(&item.label, "summary labels", 60)?;
        item.label = if label.is_empty() {
            item.key.label().to_string()
        } else {
            label
        };
    }
    t.summary_title = text(&t.summary_title, "summary heading", 60)?;
    t.table_row_label = text(&t.table_row_label, "totals row's label", 60)?;
    t.subtotal_label = text(&t.subtotal_label, "subtotal label", 60)?;
    t.opening_row_label = text(&t.opening_row_label, "opening balance row's label", 60)?;
    // Balance and type columns can drop out at printing, so something else must stay.
    if !s
        .columns
        .iter()
        .any(|c| !matches!(c.key, ColumnKey::Balance | ColumnKey::Type))
    {
        return Err(bad_request(
            "The table needs a column besides Category and Balance, e.g. Date or Description.",
        ));
    }
    s.currency_symbol = text(&s.currency_symbol, "currency symbol", 8)?;
    if s.paper != "a4" && s.paper != "us-letter" {
        return Err(bad_request("The paper size is A4 or US Letter."));
    }
    for (mm, what) in [(s.margin_top_mm, "top"), (s.margin_bottom_mm, "bottom")] {
        if !MARGINS_MM.contains(&mm) {
            return Err(bad_request(&format!(
                "The {what} margin is {} to {} mm.",
                MARGINS_MM.start(),
                MARGINS_MM.end()
            )));
        }
    }
    Ok(s)
}

/// The image type of a logo file, from its name.
pub fn logo_extension(filename: &str) -> Option<String> {
    let ext = filename.rsplit_once('.')?.1.to_ascii_lowercase();
    LOGO_EXTENSIONS.contains(&ext.as_str()).then_some(ext)
}

fn is_pdf(filename: &str) -> bool {
    filename
        .rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("pdf"))
}

/// The letterhead document's bytes and page count, or why it can't be used.
pub async fn letterhead(
    state: &AppState,
    org: &Org,
    doc: i64,
) -> Result<(Vec<u8>, usize), Response> {
    let (folder, filename) = documents::find_in_org(&state.db, org.id, doc)
        .await?
        .ok_or_else(|| bad_request("That letterhead isn't in Documents."))?;
    let bytes = documents::read_bytes(state, org, &folder, &filename, MAX_LETTERHEAD_BYTES).await?;
    let pages = statements_pdf::pdf_page_count(&bytes)
        .map_err(|why| bad_request(&format!("The letterhead can't be used: {why}.")))?;
    Ok((bytes.to_vec(), pages))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn org(db: &Db, slug: &str) -> i64 {
        db::query_scalar("INSERT INTO organisations (slug, name) VALUES (?, ?) RETURNING id")
            .bind(slug)
            .bind(slug)
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn every_org_gets_standard_and_templates_stay_in_their_org() {
        let db = db::test_db().await;
        let (a, b) = (org(&db, "a").await, org(&db, "b").await);
        let list = all(&db, a).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].settings, Settings::new("Standard", "standard"));
        // Listing again doesn't make another.
        assert_eq!(all(&db, a).await.unwrap().len(), 1);

        let theirs = all(&db, b).await.unwrap()[0].id;
        assert!(find(&db, a, theirs).await.unwrap().is_none());
        assert!(for_printing(&db, a, Some(theirs), None).await.is_err());
        // An account's last template from elsewhere (or deleted) falls back to the org's first.
        assert_eq!(
            for_printing(&db, a, None, Some(theirs)).await.unwrap().id,
            list[0].id
        );

        // Deleting a template clears it from the accounts that last used it.
        let account: i64 = db::query_scalar(
            "INSERT INTO statement_accounts (org_id, name, last_template_id) VALUES (?, 'Main', ?) RETURNING id",
        )
        .bind(a)
        .bind(list[0].id)
        .fetch_one(&db)
        .await
        .unwrap();
        db::query("DELETE FROM statement_templates WHERE id = ?")
            .bind(list[0].id)
            .execute(&db)
            .await
            .unwrap();
        let last: Option<i64> =
            db::query_scalar("SELECT last_template_id FROM statement_accounts WHERE id = ?")
                .bind(account)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(last, None);
    }

    #[tokio::test]
    async fn templates_round_trip() {
        let db = db::test_db().await;
        let a = org(&db, "a").await;
        let s = Settings::new("Bank", "bank");
        let id = insert(&db, a, &s, false).await.unwrap().unwrap();
        assert_eq!(find(&db, a, id).await.unwrap().unwrap().settings, s);
        assert!(
            insert(&db, a, &Settings::new("bank", "standard"), false)
                .await
                .is_err(),
            "names are unique ignoring case"
        );
    }

    #[tokio::test]
    async fn logos_must_be_images_and_letterheads_pdfs() {
        let db = db::test_db().await;
        let a = org(&db, "a").await;
        let doc = |name: &'static str| {
            db::query_scalar::<i64>("INSERT INTO documents (org_id, folder, filename, size) VALUES (?, '', ?, 1) RETURNING id")
                .bind(a)
                .bind(name)
        };
        let png = doc("logo.PNG").fetch_one(&db).await.unwrap();
        let pdf = doc("letterhead.pdf").fetch_one(&db).await.unwrap();
        let with = |logo, letterhead| Settings {
            logo_document_id: logo,
            letterhead_document_id: letterhead,
            ..Settings::new("T", "standard")
        };
        assert!(check(&db, a, with(Some(png), Some(pdf))).await.is_ok());
        assert!(check(&db, a, with(Some(pdf), None)).await.is_err());
        assert!(check(&db, a, with(None, Some(png))).await.is_err());
    }

    #[test]
    fn text_is_checked() {
        let base = || Settings::new("  Bank style ", "bank");
        let ok = check_text(base()).unwrap();
        assert_eq!(ok.name, "Bank style");
        let bad = |f: fn(&mut Settings)| {
            let mut s = base();
            f(&mut s);
            check_text(s).is_err()
        };
        assert!(!bad(|s| s.title = " ".into()));
        assert!(bad(|s| s.title = "For {acount.holder}".into()));
        assert!(bad(|s| s.footer = "50% {off".into()));
        assert!(bad(|s| s.layout = "fancy".into()));
        assert!(bad(|s| s.paper = "a3".into()));
        assert!(bad(|s| s.margin_bottom_mm = 2));
        assert!(bad(|s| s.header_fields.push(Field {
            label: "X".into(),
            value: "{nope}".into()
        })));
        assert!(bad(|s| s.header_fields = vec![
            Field {
                label: "X".into(),
                value: "y".into()
            };
            21
        ]));
        assert!(bad(|s| s.columns.push(Column {
            key: ColumnKey::Date,
            label: "Again".into()
        })));
        assert!(bad(|s| s.columns.retain(|c| c.key == ColumnKey::Balance)));
        assert!(bad(|s| s.columns[0].label = "x".repeat(41)));
        // A blank heading becomes the usual one.
        let mut s = base();
        s.columns[2].label = "  ".into();
        assert_eq!(check_text(s).unwrap().columns[2].label, "Debit");
        assert!(bad(|s| s.totals.summary.push(s.totals.summary[0].clone())));
        assert!(bad(|s| s.totals.table_row_label = "Total {nope}".into()));
        let mut s = base();
        s.totals.summary[0].label = " ".into();
        assert_eq!(
            check_text(s).unwrap().totals.summary[0].label,
            "Opening balance"
        );
        // Empty rows are dropped.
        let mut s = base();
        s.header_fields.push(Field {
            label: " ".into(),
            value: "".into(),
        });
        assert_eq!(check_text(s).unwrap().header_fields.len(), 6);
    }
}
