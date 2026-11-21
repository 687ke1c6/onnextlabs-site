//! statements-iq core: read bank statements, give rows their n8n-compatible ids and categories, and append them to
//! a Google Sheets tab. Used by the CLI and the web server.

pub mod categories;
pub mod google;
pub mod hash;
pub mod oauth;
pub mod row;
pub mod sb_csv;
pub mod statement;
pub mod upload;
