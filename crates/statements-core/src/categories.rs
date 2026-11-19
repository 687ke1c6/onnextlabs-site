//! Type rules: fill the `type` column from the description (and optionally the amount), like the n8n
//! "Add SB Catagories (type)" node. Rules are tried in order; the first match wins; no match leaves `type`
//! blank. The rules belong to the caller (the portal keeps them per organisation); build each with
//! [`CategoryRule::new`], which validates it.

use anyhow::{Result, ensure};

use crate::statement::{Format, Statement, Transaction};

#[derive(Debug, Clone, PartialEq)]
pub struct CategoryRule {
    /// The value written to `type`.
    pub category: String,
    /// Matches when the description contains any of these (case-insensitive). Stored upper-cased.
    pub contains: Vec<String>,
    /// Amount must be >= this.
    pub min_amount: Option<f64>,
    /// Amount must be < this.
    pub max_amount: Option<f64>,
    /// Only for these statement formats (empty: any).
    pub formats: Vec<Format>,
}

impl CategoryRule {
    /// A checked rule. Surrounding whitespace is trimmed from `category`; `contains` is kept as given apart from
    /// upper-casing (leading and trailing spaces in a needle are meaningful).
    pub fn new(
        category: &str,
        contains: &[String],
        min_amount: Option<f64>,
        max_amount: Option<f64>,
        formats: Vec<Format>,
    ) -> Result<Self> {
        let category = category.trim().to_string();
        ensure!(!category.is_empty(), "the type is empty");
        ensure!(!contains.is_empty(), "give at least one text to look for");
        // An empty needle would match every description.
        ensure!(
            contains.iter().all(|c| !c.is_empty()),
            "a text to look for is empty"
        );
        for amount in [min_amount, max_amount].into_iter().flatten() {
            ensure!(amount.is_finite(), "amounts must be numbers");
        }
        if let (Some(min), Some(max)) = (min_amount, max_amount) {
            ensure!(min < max, "the minimum amount must be below the maximum");
        }
        Ok(CategoryRule {
            category,
            contains: contains.iter().map(|c| c.to_uppercase()).collect(),
            min_amount,
            max_amount,
            formats,
        })
    }

    /// Whether this rule sets the type of `t`, a transaction from a statement in `format`.
    pub fn matches(&self, t: &Transaction, format: Format) -> bool {
        if !self.formats.is_empty() && !self.formats.contains(&format) {
            return false;
        }
        // n8n: `description.toUpperCase().includes(needle)`.
        let description = t.description.to_uppercase();
        if !self
            .contains
            .iter()
            .any(|needle| description.contains(needle.as_str()))
        {
            return false;
        }
        if self.min_amount.is_none() && self.max_amount.is_none() {
            return true;
        }
        // n8n: `parseFloat(amount.replace(/,/g, ""))`; an unreadable amount fails any amount condition.
        let Ok(amount) = t.amount.replace(',', "").trim().parse::<f64>() else {
            return false;
        };
        self.min_amount.is_none_or(|min| amount >= min)
            && self.max_amount.is_none_or(|max| amount < max)
    }
}

/// Sets each transaction's category from the first matching rule (or none). Returns how many transactions got a
/// category.
pub fn categorise(rules: &[CategoryRule], statement: &mut Statement) -> usize {
    let format = statement.format;
    let mut categorised = 0;
    for t in &mut statement.transactions {
        t.category = rules
            .iter()
            .find(|r| r.matches(t, format))
            .map(|r| r.category.clone());
        categorised += usize::from(t.category.is_some());
    }
    categorised
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(category: &str, contains: &[&str]) -> CategoryRule {
        CategoryRule {
            category: category.into(),
            contains: contains.iter().map(|s| s.to_uppercase()).collect(),
            min_amount: None,
            max_amount: None,
            formats: vec![],
        }
    }

    fn statement(rows: &[(&str, &str)]) -> Statement {
        Statement {
            format: Format::SbCsv,
            account_number: None,
            transactions: rows
                .iter()
                .map(|(description, amount)| Transaction {
                    id: 0,
                    date: "2027-01-01".into(),
                    description: (*description).into(),
                    amount: (*amount).into(),
                    currency: "ZAR".into(),
                    category: None,
                })
                .collect(),
        }
    }

    fn categories(s: &Statement) -> Vec<Option<&str>> {
        s.transactions
            .iter()
            .map(|t| t.category.as_deref())
            .collect()
    }

    #[test]
    fn first_matching_rule_wins_case_insensitively() {
        let rules = [
            rule("BOND", &["home loan"]),
            rule("FEES", &["FEE"]),
            rule("LOAN", &["LOAN"]),
        ];
        let mut s = statement(&[
            ("HOME LOAN DEBIT", "-1"),
            ("monthly fee ", "-1"),
            ("COFFEE SHOP", "-1"), // n8n's plain substring match: FEE is in COFFEE
            ("SALARY", "1"),
        ]);
        assert_eq!(categorise(&rules, &mut s), 3);
        assert_eq!(
            categories(&s),
            [Some("BOND"), Some("FEES"), Some("FEES"), None]
        );
    }

    #[test]
    fn amount_bounds_are_min_inclusive_max_exclusive() {
        let mut rent = rule("RENT", &["TENANT"]);
        rent.min_amount = Some(38_000.0);
        let mut utilities = rule("UTILITIES", &["TENANT"]);
        utilities.max_amount = Some(10_000.0);
        let mut s = statement(&[
            ("TENANT", "38000"),
            ("TENANT", "37999.99"),
            ("TENANT", "9999.99"),
            ("TENANT", "10000"),
            ("TENANT", "40,000.00"),
            ("TENANT", "n/a"),
        ]);
        categorise(&[rent, utilities], &mut s);
        assert_eq!(
            categories(&s),
            [
                Some("RENT"),
                None,
                Some("UTILITIES"),
                None,
                Some("RENT"),
                None
            ]
        );
    }

    #[test]
    fn rules_can_be_limited_to_formats() {
        let mut s = statement(&[("X", "1")]);
        let mut sb_only = rule("SB", &["X"]);
        sb_only.formats = vec![Format::SbCsv];
        categorise(&[sb_only], &mut s);
        assert_eq!(categories(&s), [Some("SB")]);
    }

    #[test]
    fn new_trims_the_type_and_upper_cases_needles() {
        let r = CategoryRule::new(" RENT ", &["tenant ".into()], Some(1.0), None, vec![]).unwrap();
        assert_eq!(r.category, "RENT");
        assert_eq!(r.contains, ["TENANT "]);
    }

    #[test]
    fn rejects_bad_rules() {
        let bad = |category: &str, contains: &[&str], min: Option<f64>, max: Option<f64>| {
            let contains: Vec<String> = contains.iter().map(|c| c.to_string()).collect();
            CategoryRule::new(category, &contains, min, max, vec![])
                .unwrap_err()
                .to_string()
        };
        assert!(bad("X", &[], None, None).contains("at least one"));
        assert!(bad("X", &[""], None, None).contains("is empty"));
        assert!(bad(" ", &["A"], None, None).contains("type is empty"));
        assert!(bad("X", &["A"], Some(5.0), Some(5.0)).contains("below the maximum"));
        assert!(bad("X", &["A"], Some(f64::NAN), None).contains("numbers"));
    }
}
