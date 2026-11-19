//! Minimal Google Drive / Sheets REST client: list and create spreadsheets and tabs, read and append values.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

const DRIVE_FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const SHEETS_URL: &str = "https://sheets.googleapis.com/v4/spreadsheets";

/// An HTTP agent that returns 4xx/5xx responses instead of failing, so Google's error message can be shown.
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

pub struct Google {
    agent: ureq::Agent,
    access_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spreadsheet {
    pub id: String,
    pub name: String,
    /// RFC 3339, e.g. `2026-11-18T09:12:33.000Z`.
    #[serde(default)]
    pub modified_time: String,
}

/// A spreadsheet's title and tabs, from the Sheets API.
#[derive(Debug, Clone)]
pub struct SpreadsheetDetails {
    pub title: String,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone)]
pub struct Tab {
    /// Numeric tab id (the `gid` in the spreadsheet URL).
    pub sheet_id: u64,
    pub title: String,
    /// Grid size, including empty rows (not the number of filled rows).
    pub row_count: u64,
}

impl Google {
    pub fn new(agent: ureq::Agent, access_token: String) -> Self {
        Self {
            agent,
            access_token,
        }
    }

    /// Spreadsheets the signed-in user can see, most recently modified first (trash excluded).
    pub fn list_spreadsheets(&self) -> Result<Vec<Spreadsheet>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            #[serde(default)]
            files: Vec<Spreadsheet>,
            next_page_token: Option<String>,
        }

        let mut all = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let request = || {
                let request = self
                    .agent
                    .get(DRIVE_FILES_URL)
                    .query(
                        "q",
                        "mimeType='application/vnd.google-apps.spreadsheet' and trashed=false",
                    )
                    .query("fields", "nextPageToken,files(id,name,modifiedTime)")
                    .query("orderBy", "modifiedTime desc")
                    .query("pageSize", "100");
                match &page_token {
                    Some(token) => request.query("pageToken", token),
                    None => request,
                }
            };
            let page: Page = self
                .send(request)
                .context("listing spreadsheets (Drive API)")?;
            all.extend(page.files);
            match page.next_page_token {
                Some(token) => page_token = Some(token),
                None => return Ok(all),
            }
        }
    }

    /// A spreadsheet's title and its tabs, in sheet order (one Sheets API read).
    pub fn spreadsheet(&self, spreadsheet_id: &str) -> Result<SpreadsheetDetails> {
        #[derive(Deserialize)]
        struct Response {
            properties: Title,
            #[serde(default)]
            sheets: Vec<Sheet>,
        }
        #[derive(Deserialize)]
        struct Title {
            title: String,
        }

        let request = || {
            self.agent
                .get(format!("{SHEETS_URL}/{spreadsheet_id}"))
                .query(
                    "fields",
                    format!("properties.title,sheets.properties({TAB_FIELDS})"),
                )
        };
        let response: Response = self
            .send(request)
            .with_context(|| format!("reading spreadsheet {spreadsheet_id} (Sheets API)"))?;
        Ok(SpreadsheetDetails {
            title: response.properties.title,
            tabs: response.sheets.into_iter().map(Tab::from).collect(),
        })
    }

    /// Creates a spreadsheet in the root of your Drive whose only tab is `tab_title`. Returns its id and the tab.
    pub fn create_spreadsheet(&self, title: &str, tab_title: &str) -> Result<(String, Tab)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Response {
            spreadsheet_id: String,
            sheets: Vec<Sheet>,
        }

        let body = json!({
            "properties": { "title": title },
            "sheets": [{ "properties": { "title": tab_title } }],
        });
        let request = || {
            self.agent.post(SHEETS_URL).query(
                "fields",
                format!("spreadsheetId,sheets.properties({TAB_FIELDS})"),
            )
        };
        let response: Response = self
            .send_json(request, &body)
            .with_context(|| format!("creating spreadsheet {title:?} (Sheets API)"))?;
        let tab = response
            .sheets
            .into_iter()
            .next()
            .map(Tab::from)
            .context("Google created the spreadsheet without a tab")?;
        Ok((response.spreadsheet_id, tab))
    }

    /// Adds a tab at the end of a spreadsheet.
    pub fn add_tab(&self, spreadsheet_id: &str, title: &str) -> Result<Tab> {
        #[derive(Deserialize)]
        struct Response {
            replies: Vec<Reply>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Reply {
            add_sheet: Sheet,
        }

        let body = json!({ "requests": [{ "addSheet": { "properties": { "title": title } } }] });
        let request = || {
            self.agent
                .post(format!("{SHEETS_URL}/{spreadsheet_id}:batchUpdate"))
                .query(
                    "fields",
                    format!("replies.addSheet.properties({TAB_FIELDS})"),
                )
        };
        let response: Response = self
            .send_json(request, &body)
            .with_context(|| format!("adding tab {title:?} (Sheets API)"))?;
        let reply = response
            .replies
            .into_iter()
            .next()
            .context("Google added the tab without a reply")?;
        Ok(Tab::from(reply.add_sheet))
    }

    /// Values in an A1 range (e.g. `'Tab'!1:1`), row by row, unformatted: numbers come back as JSON numbers,
    /// text as strings. Trailing empty rows and cells are left out, as the Sheets API does.
    pub fn read_values(
        &self,
        spreadsheet_id: &str,
        range: &str,
    ) -> Result<Vec<Vec<serde_json::Value>>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Response {
            #[serde(default)]
            value_ranges: Vec<ValueRange>,
        }
        #[derive(Deserialize)]
        struct ValueRange {
            #[serde(default)]
            values: Vec<Vec<serde_json::Value>>,
        }

        // batchGet takes the range as a query parameter, so tab names need no path escaping.
        let request = || {
            self.agent
                .get(format!("{SHEETS_URL}/{spreadsheet_id}/values:batchGet"))
                .query("ranges", range)
                .query("majorDimension", "ROWS")
                .query("valueRenderOption", "UNFORMATTED_VALUE")
        };
        let response: Response = self
            .send(request)
            .with_context(|| format!("reading {range} (Sheets API)"))?;
        Ok(response
            .value_ranges
            .into_iter()
            .next()
            .map(|r| r.values)
            .unwrap_or_default())
    }

    /// Appends rows after the table found at `range`, inserting new rows so no existing cell is overwritten.
    /// Values are entered as if typed (`USER_ENTERED`). Returns the number of rows written.
    pub fn append_rows(
        &self,
        spreadsheet_id: &str,
        range: &str,
        rows: Vec<Vec<serde_json::Value>>,
    ) -> Result<u64> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Response {
            updates: Updates,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Updates {
            #[serde(default)]
            updated_rows: u64,
        }

        let body = json!({ "majorDimension": "ROWS", "values": rows });
        let encoded_range = utf8_percent_encode(range, NON_ALPHANUMERIC);
        let request = || {
            self.agent
                .post(format!(
                    "{SHEETS_URL}/{spreadsheet_id}/values/{encoded_range}:append"
                ))
                .query("valueInputOption", "USER_ENTERED")
                .query("insertDataOption", "INSERT_ROWS")
                .query("fields", "updates.updatedRows")
        };
        let response: Response = self
            .send_json(request, &body)
            .with_context(|| format!("appending rows to {range} (Sheets API)"))?;
        Ok(response.updates.updated_rows)
    }

    /// Sends a GET with the access token, retrying rate limits and transient server errors.
    fn send<T: DeserializeOwned>(
        &self,
        request: impl Fn() -> ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    ) -> Result<T> {
        let auth = format!("Bearer {}", self.access_token);
        let mut response = with_retry(is_retryable, || {
            request().header("Authorization", &auth).call()
        })?;
        Ok(response.body_mut().read_json()?)
    }

    /// Sends a JSON POST with the access token. Only rate limits (429, which Google rejects before doing anything)
    /// are retried: after a 5xx the create may have happened, and retrying could make a duplicate.
    fn send_json<T: DeserializeOwned>(
        &self,
        request: impl Fn() -> ureq::RequestBuilder<ureq::typestate::WithBody>,
        body: &serde_json::Value,
    ) -> Result<T> {
        let auth = format!("Bearer {}", self.access_token);
        let mut response = with_retry(
            |status| status == 429,
            || request().header("Authorization", &auth).send_json(body),
        )?;
        Ok(response.body_mut().read_json()?)
    }
}

/// Fields requested for each tab.
const TAB_FIELDS: &str = "sheetId,title,gridProperties.rowCount";

/// A tab as the Sheets API returns it (`{ properties: { title, gridProperties: { rowCount } } }`).
#[derive(Deserialize)]
struct Sheet {
    properties: SheetProperties,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SheetProperties {
    #[serde(default)]
    sheet_id: u64,
    title: String,
    #[serde(default)]
    grid_properties: Option<GridProperties>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GridProperties {
    #[serde(default)]
    row_count: u64,
}

impl From<Sheet> for Tab {
    fn from(sheet: Sheet) -> Self {
        Tab {
            sheet_id: sheet.properties.sheet_id,
            title: sheet.properties.title,
            row_count: sheet.properties.grid_properties.map_or(0, |g| g.row_count),
        }
    }
}

/// Delays before each retry. Google's quotas are per minute (e.g. 60 Sheets reads per user), so the waits add up
/// to a little over a minute before giving up.
const RETRY_DELAYS_SECS: [u64; 6] = [1, 2, 4, 8, 16, 32];

/// Runs `call`, retrying statuses `retryable` accepts (honouring `Retry-After` when Google sends it). Any other
/// non-success status becomes an error carrying Google's message.
fn with_retry(
    retryable: fn(u16) -> bool,
    mut call: impl FnMut() -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<ureq::http::Response<ureq::Body>> {
    let mut delays = RETRY_DELAYS_SECS.into_iter();
    loop {
        let mut response = call()?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if retryable(status.as_u16())
            && let Some(default_delay) = delays.next()
        {
            let delay = retry_after_secs(&response).unwrap_or(default_delay);
            eprintln!("Google answered {status}; retrying in {delay}s ...");
            std::thread::sleep(Duration::from_secs(delay));
            continue;
        }
        let body = response.body_mut().read_to_string().unwrap_or_default();
        return Err(anyhow!("{status}: {}", google_error_message(&body)));
    }
}

fn is_retryable(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// `Retry-After` in seconds (Google uses the seconds form), capped so a bad value can't hang the CLI.
fn retry_after_secs(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    let secs: u64 = response
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(secs.min(120))
}

/// Google's `{"error": {"message": ...}}`, or the raw body when it isn't that shape. The message names the cause,
/// e.g. that an API is not enabled for the project, with the console link to enable it.
fn google_error_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct Envelope {
        error: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        message: String,
    }
    serde_json::from_str::<Envelope>(body)
        .map(|e| e.error.message)
        .unwrap_or_else(|_| body.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_only_rate_limits_and_server_errors() {
        for status in [429, 500, 502, 503, 504] {
            assert!(is_retryable(status), "{status}");
        }
        for status in [400, 401, 403, 404] {
            assert!(!is_retryable(status), "{status}");
        }
    }

    #[test]
    fn parses_created_tab() {
        let sheet: Sheet = serde_json::from_str(
            r#"{"properties":{"sheetId":42,"title":"Transactions","gridProperties":{"rowCount":1000}}}"#,
        )
        .unwrap();
        let tab = Tab::from(sheet);
        assert_eq!(
            (tab.sheet_id, tab.title.as_str(), tab.row_count),
            (42, "Transactions", 1000)
        );
    }

    #[test]
    fn extracts_google_error_message() {
        let body = r#"{"error":{"code":403,"message":"Google Sheets API has not been used in project 1 before or it is disabled.","status":"PERMISSION_DENIED"}}"#;
        assert_eq!(
            google_error_message(body),
            "Google Sheets API has not been used in project 1 before or it is disabled."
        );
        assert_eq!(google_error_message("  Bad gateway \n"), "Bad gateway");
    }
}
