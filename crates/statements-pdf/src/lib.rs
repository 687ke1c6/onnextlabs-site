//! Statement PDFs: a [Typst](https://typst.app) template plus the statement's data, rendered to
//! PDF in process (no browser, no network).
//!
//! The template is Typst source. It reads the statement from `/data.json` (see [`StatementData`])
//! and may use the assets passed with it (e.g. `/logo.png`). [`DEFAULT_TEMPLATE`] is the built-in
//! one, laid out for official use: who and what the statement is for, the period and selection,
//! totals, a summary by type, and every row. [`BANK_TEMPLATE`] is laid out like a bank's.
//!
//! Text the template's author types (title, header fields, footer) can use `{name}` variables
//! ([`vars`]); [`build`] fills them in, so templates only lay out ready-made strings.
//!
//! A PDF can be passed as an asset too (e.g. `/letterhead.pdf`): the default template draws it
//! behind the pages (see [`Letterhead`]).
//!
//! Fonts are Typst's bundled ones (Libertinus Serif, New Computer Modern, DejaVu Sans Mono).

use std::{collections::HashMap, sync::LazyLock};

use typst::{
    Library, LibraryExt, World, WorldExt,
    diag::{FileError, FileResult, SourceDiagnostic},
    foundations::{Bytes, Datetime},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_layout::PagedDocument;

mod data;
pub mod vars;
pub use data::*;

/// The built-in statement template.
pub const DEFAULT_TEMPLATE: &str = include_str!("statement.typ");
/// A built-in template laid out like a bank's statement: the header fields beside a balance
/// summary, then money out, money in and (with a starting balance) the balance on each row.
pub const BANK_TEMPLATE: &str = include_str!("bank.typ");

/// The built-in layouts: `(name, label)`.
pub const LAYOUTS: [(&str, &str); 2] = [("standard", "Standard"), ("bank", "Bank style")];

/// The Typst source of a built-in layout (`standard` for anything unknown).
pub fn layout_source(layout: &str) -> &'static str {
    match layout {
        "bank" => BANK_TEMPLATE,
        _ => DEFAULT_TEMPLATE,
    }
}

/// A file the template can read, e.g. `("logo.png", bytes)`, at `/logo.png`.
pub struct Asset {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// How many pages a PDF has, e.g. to check a letterhead before using it. `Err` says why it can't
/// be used (not a PDF, damaged, password-protected, or no pages).
pub fn pdf_page_count(bytes: &[u8]) -> Result<usize, String> {
    let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.to_vec())).map_err(|e| match e {
        hayro_syntax::LoadPdfError::Decryption(_) => "it's password-protected".to_string(),
        _ => "it isn't a PDF that can be read".to_string(),
    })?;
    match pdf.pages().len() {
        0 => Err("it has no pages".into()),
        n => Ok(n),
    }
}

/// Why a statement couldn't be rendered: the template's errors, each with its line when known.
#[derive(Debug)]
pub struct RenderError(pub Vec<String>);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("; "))
    }
}

impl std::error::Error for RenderError {}

/// Renders `template` with `data` (and `assets`) to PDF bytes. CPU-bound: call it off the async
/// runtime for big statements.
pub fn render(
    template: &str,
    data: &StatementData,
    assets: Vec<Asset>,
) -> Result<Vec<u8>, RenderError> {
    let world = world(template, data, assets)?;
    let document = compile_world(&world)?;
    typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
        .map_err(|errors| RenderError(messages(&world, &errors)))
}

/// The laid-out pages (for previews in tests).
#[cfg(test)]
fn pages(
    template: &str,
    data: &StatementData,
    assets: Vec<Asset>,
) -> Result<PagedDocument, RenderError> {
    compile_world(&world(template, data, assets)?)
}

fn compile_world(world: &StatementWorld) -> Result<PagedDocument, RenderError> {
    typst::compile::<PagedDocument>(world)
        .output
        .map_err(|errors| RenderError(messages(world, &errors)))
}

fn world(
    template: &str,
    data: &StatementData,
    assets: Vec<Asset>,
) -> Result<StatementWorld, RenderError> {
    let json =
        serde_json::to_vec(data).map_err(|e| RenderError(vec![format!("statement data: {e}")]))?;
    let mut files = HashMap::new();
    files.insert(file_id("/data.json"), Bytes::new(json));
    for a in assets {
        // Only plain names: the template sees them at the root.
        if a.name.is_empty() || a.name.contains(['/', '\\']) || a.name.starts_with('.') {
            return Err(RenderError(vec![format!("bad asset name {:?}", a.name)]));
        }
        files.insert(file_id(&format!("/{}", a.name)), Bytes::new(a.bytes));
    }
    Ok(StatementWorld {
        main: Source::new(file_id("/main.typ"), template.to_string()),
        files,
        today: today(&data.generated),
    })
}

/// File ids are interned for the life of the process (at most 2^16), so only ever a few fixed
/// paths are used: `/main.typ`, `/data.json` and the asset names.
fn file_id(path: &str) -> FileId {
    FileId::new(RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new(path).expect("fixed paths are valid"),
    ))
}

/// `YYYY-MM-DD` as Typst's today (for `datetime.today()`); None if it isn't one.
fn today(date: &str) -> Option<Datetime> {
    let mut parts = date.splitn(3, '-').map(|p| p.parse::<u32>().ok());
    let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Datetime::from_ymd(
        i32::try_from(y).ok()?,
        u8::try_from(m).ok()?,
        u8::try_from(d).ok()?,
    )
}

/// "line 12: unknown variable: foo" (with any hints), for each error.
fn messages(world: &StatementWorld, errors: &[SourceDiagnostic]) -> Vec<String> {
    errors
        .iter()
        .map(|e| {
            let line = e
                .span
                .id()
                .filter(|id| *id == world.main.id())
                .and_then(|_| world.range(e.span))
                .map(|r| world.main.text()[..r.start].matches('\n').count() + 1);
            let mut msg = match line {
                Some(l) => format!("line {l}: {}", e.message),
                None => e.message.to_string(),
            };
            for hint in &e.hints {
                msg.push_str(&format!(" (hint: {})", hint.v));
            }
            msg
        })
        .collect()
}

/// The fonts, parsed once.
static FONTS: LazyLock<(LazyHash<FontBook>, Vec<Font>)> = LazyLock::new(|| {
    let fonts: Vec<Font> = typst_assets::fonts()
        .flat_map(|data| Font::iter(Bytes::new(data)))
        .collect();
    (LazyHash::new(FontBook::from_fonts(&fonts)), fonts)
});

static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| LazyHash::new(Library::default()));

struct StatementWorld {
    main: Source,
    files: HashMap<FileId, Bytes>,
    today: Option<Datetime>,
}

impl World for StatementWorld {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &FONTS.0
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::NotSource)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main.id() {
            return Ok(Bytes::from_string(self.main.clone()));
        }
        self.files
            .get(&id)
            .cloned()
            .ok_or_else(|| FileError::NotFound(id.vpath().get_with_slash().into()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        FONTS.1.get(index).cloned()
    }

    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        self.today
    }
}

#[cfg(test)]
mod tests;
