//! `{name}` variables in a template's text (title, subtitle, header fields, footer), filled in
//! before the text reaches Typst, so whatever is typed prints as plain text.
//!
//! `{{` and `}}` print a literal brace. An unknown name, or a `{` with no `}`, is an error, which
//! [`check`] reports when a template is saved.

use std::collections::HashMap;

/// Every variable, with what it prints (for the template editor).
pub const VARIABLES: [(&str, &str); 16] = [
    ("org.name", "The organisation's name"),
    ("account.name", "The bank account's name"),
    (
        "account.number",
        "The bank account's identifier, e.g. the account number",
    ),
    ("account.holder", "The bank account holder"),
    ("account.product", "The bank account's product name"),
    ("currency", "The rows' currency, e.g. ZAR"),
    ("period.from", "The first day of the statement period"),
    ("period.to", "The last day of the statement period"),
    ("balance.opening", "The balance at the start of the period"),
    ("balance.closing", "The balance at the end of the period"),
    ("total.in", "Money in, of the printed rows"),
    ("total.out", "Money out, of the printed rows"),
    ("total.net", "Money in less money out, of the printed rows"),
    ("count", "How many rows are printed"),
    (
        "selection",
        "The filters, in words (or \"All transactions in the bank account\")",
    ),
    ("generated", "The day it was printed"),
];

/// Why a text can't be filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FillError {
    Unknown(String),
    Unclosed,
}

impl std::fmt::Display for FillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FillError::Unknown(name) => write!(f, "there's no variable {{{name}}}"),
            FillError::Unclosed => f.write_str("a { has no closing }; type {{ for a brace"),
        }
    }
}

/// `text` with each `{name}` replaced by its value in `vars`.
pub fn fill(text: &str, vars: &HashMap<&str, String>) -> Result<String, FillError> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => name.push(c),
                        None => return Err(FillError::Unclosed),
                    }
                }
                let name = name.trim();
                match vars.get(name) {
                    Some(value) => out.push_str(value),
                    None => return Err(FillError::Unknown(name.to_string())),
                }
            }
            c => out.push(c),
        }
    }
    Ok(out)
}

/// Whether `text` would fill in: its variables all exist and its braces are closed.
pub fn check(text: &str) -> Result<(), FillError> {
    let placeholders = VARIABLES.iter().map(|(n, _)| (*n, String::new())).collect();
    fill(text, &placeholders).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_escapes_and_refuses() {
        let vars = HashMap::from([("account.holder", "Mr J Smith".to_string())]);
        assert_eq!(
            fill("Holder: {account.holder}", &vars).unwrap(),
            "Holder: Mr J Smith"
        );
        assert_eq!(fill("{ account.holder }", &vars).unwrap(), "Mr J Smith");
        assert_eq!(fill("{{literal}} and }", &vars).unwrap(), "{literal} and }");
        assert_eq!(
            fill("{nope}", &vars),
            Err(FillError::Unknown("nope".into()))
        );
        assert_eq!(fill("oops {account", &vars), Err(FillError::Unclosed));
        assert!(check("{period.from} - {period.to}").is_ok());
        assert!(check("{period.start}").is_err());
    }
}
