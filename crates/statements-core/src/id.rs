//! Row ids. A row's id is a hash of its *key*: the date, the amount in cents, the description with formatting
//! noise removed, and the row's occurrence (0, 1, 2... for rows of one statement with the same key, in file
//! order). The values themselves are stored exactly as the file gives them; only the key is normalised, so the
//! same transaction gets the same id whichever format it arrives in:
//!
//! - amounts: `-1798`, `-1798.00` and `-1,798.00` are all `-179800` cents;
//! - descriptions: `##` (the fee column of Standard Bank PDF statements, run into the description) is dropped,
//!   runs of whitespace become one space, and the ends are trimmed. Case is kept.
//!
//! Counting occurrences on the normalised key means two rows of one file never share an id, however similar.
//! Like n8n's `_` suffix, it assumes a statement holds every transaction of each day it covers.
//!
//! Ids are 48 bits: exact as JSON numbers in JavaScript (53 bits) and in Google Sheets (15 significant digits).
//! (The n8n ids these replace are in `hash.rs`; they're still checked when a Google Sheet export is imported.)

use anyhow::{Result, bail, ensure};
use sha2::{Digest, Sha256};

/// Largest id + 1.
pub const ID_LIMIT: i64 = 1 << 48;

/// The id of a row with this key. `date` is `YYYY-MM-DD`; `description` is already normalised
/// ([`key_description`]).
pub fn row_id(date: &str, cents: i64, description: &str, occurrence: u32) -> i64 {
    // Fields separated by a character no field contains (descriptions have no control characters once
    // normalised), so different keys can't produce the same input.
    let digest = Sha256::digest(format!(
        "{date}\x1f{cents}\x1f{description}\x1f{occurrence}"
    ));
    let mut bytes = [0u8; 8];
    bytes[2..].copy_from_slice(&digest[..6]);
    i64::from_be_bytes(bytes)
}

/// The id of the `n`th row of a split of the row with id `parent` (rows added by hand to move part of an amount
/// to another type or date). A different input shape from [`row_id`]'s, so it can never be a statement row's id.
pub fn split_row_id(parent: i64, n: u32) -> i64 {
    let digest = Sha256::digest(format!("split\x1f{parent}\x1f{n}"));
    let mut bytes = [0u8; 8];
    bytes[2..].copy_from_slice(&digest[..6]);
    i64::from_be_bytes(bytes)
}

/// Cents as an amount string with two decimals: `-472616` -> `-4726.16`.
pub fn format_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// The amount in cents: an optional sign, digits (`,` thousands separators allowed), and at most two decimals.
pub fn amount_cents(amount: &str) -> Result<i64> {
    let plain: String = amount.trim().chars().filter(|&c| c != ',').collect();
    let (negative, digits) = match plain.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, plain.strip_prefix('+').unwrap_or(&plain)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    ensure!(
        !(whole.is_empty() && fraction.is_empty()) && all_digits(whole) && all_digits(fraction),
        "amount {amount:?} isn't a number"
    );
    if fraction.len() > 2 {
        bail!("amount {amount:?} has more than two decimals");
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse()? };
    let fraction: i64 = format!("{fraction:0<2}").parse()?;
    let cents = whole
        .checked_mul(100)
        .and_then(|c| c.checked_add(fraction))
        .ok_or_else(|| anyhow::anyhow!("amount {amount:?} is too large"))?;
    Ok(if negative { -cents } else { cents })
}

/// The description as it goes into the key: `##` dropped, whitespace runs (and control characters) made one
/// space, ends trimmed. Case is kept.
pub fn key_description(description: &str) -> String {
    description
        .replace("##", " ")
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_in_any_format_are_the_same_cents() {
        for a in ["-1798", "-1798.00", "-1,798.00", " -1798.0 ", "-1,798"] {
            assert_eq!(amount_cents(a).unwrap(), -179_800, "{a}");
        }
        assert_eq!(amount_cents("38,000.00").unwrap(), 3_800_000);
        assert_eq!(amount_cents("+5.5").unwrap(), 550);
        assert_eq!(amount_cents(".5").unwrap(), 50);
        assert_eq!(amount_cents("0.00").unwrap(), 0);
        assert_eq!(amount_cents("-0.01").unwrap(), -1);
    }

    #[test]
    fn unreadable_amounts_are_refused() {
        for a in [
            "",
            "-",
            ".",
            "R 12",
            "1.234",
            "1e5",
            "--1",
            "12-",
            "1.2.3",
            "99999999999999999999",
        ] {
            assert!(amount_cents(a).is_err(), "{a:?}");
        }
    }

    /// Real pairs: the same transaction from a Standard Bank PDF statement and from the CSV export.
    #[test]
    fn descriptions_from_either_format_give_the_same_key() {
        for (pdf, csv) in [
            (
                "IB PAYMENT TO CITY OF CAPE TOWN RATES 29 ST",
                "IB PAYMENT TO CITY OF CAPE TOWN  RATES 29 ST",
            ),
            (
                "FEE-ELECTRONIC ACCOUNT PAYMENT ## 10197691372",
                "FEE-ELECTRONIC ACCOUNT PAYMENT 10197691372",
            ),
            (
                "STD BANK BOND REPAYMENT SBSA HOMEL 536771472 241028",
                "STD BANK BOND REPAYMENT SBSA HOMEL 536771472    241028",
            ),
            ("FEE - DEBIT ORDER ##", "FEE - DEBIT ORDER "),
            (
                "CREDIT TRANSFER INVESTECPB29STMICHAELS OCT UTI ",
                "CREDIT TRANSFER INVESTECPB29STMICHAELS OCT UTI",
            ),
        ] {
            assert_eq!(key_description(pdf), key_description(csv));
        }
        assert_eq!(key_description("FEE##X"), "FEE X");
        assert_ne!(
            key_description("Yoco *Green"),
            key_description("YOCO *GREEN")
        );
    }

    #[test]
    fn ids_are_48_bit_and_depend_on_every_part_of_the_key() {
        let id = row_id("2028-01-01", -100, "FEE", 0);
        assert!((0..ID_LIMIT).contains(&id));
        assert_eq!(id, row_id("2028-01-01", -100, "FEE", 0));
        for other in [
            row_id("2028-01-02", -100, "FEE", 0),
            row_id("2028-01-01", 100, "FEE", 0),
            row_id("2028-01-01", -100, "FEE ", 0),
            row_id("2028-01-01", -100, "FEE", 1),
        ] {
            assert_ne!(id, other);
        }
        // Pinned: changing the key or the hash changes every stored id.
        assert_eq!(row_id("2028-01-01", -100, "FEE", 0), PINNED);
    }

    #[test]
    fn split_ids_are_48_bit_and_per_parent_and_position() {
        let id = split_row_id(PINNED, 0);
        assert!((0..ID_LIMIT).contains(&id));
        assert_ne!(id, split_row_id(PINNED, 1));
        assert_ne!(id, split_row_id(PINNED + 1, 0));
    }

    #[test]
    fn cents_format_back_to_amounts() {
        for (cents, s) in [
            (-472_616, "-4726.16"),
            (5, "0.05"),
            (-5, "-0.05"),
            (0, "0.00"),
            (3_800_000, "38000.00"),
        ] {
            assert_eq!(format_cents(cents), s);
            assert_eq!(amount_cents(s).unwrap(), cents);
        }
    }

    const PINNED: i64 = 106_685_447_381_696;
}
