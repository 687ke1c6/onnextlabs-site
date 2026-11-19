//! The JSON the database service speaks.
//!
//! Two requests:
//!
//! - `POST /v1/query` with a [`Statement`] runs one statement on its own and answers with a
//!   [`StmtResult`].
//! - `POST /v1/batch` with a [`Batch`] runs its statements in order in one transaction —
//!   all apply or none do — and answers with a [`BatchResult`].
//!
//! Failures answer with an [`ErrorBody`]. There are no transactions spanning requests:
//! anything that must be atomic goes in one batch, and later statements find what earlier
//! ones did by subquery, not by the caller reading results in between.
//!
//! Values ([`Value`]) map to JSON as: NULL → `null`, INTEGER → a number without a fraction,
//! REAL → a number with one (`2.0`), TEXT → a string, BLOB → `{"base64": "..."}`.

use std::borrow::Cow;
use std::fmt;

use base64::Engine;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::{run_batch, run_statement};

// ---- values ----

/// A SQLite value, as bound to a statement or read from a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn into_string(self) -> Option<String> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(v: $t) -> Self {
                Value::Integer(v as i64)
            }
        }
    )*};
}
from_int!(i64, i32, u32, usize);

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Integer(v.into())
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Real(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_owned())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}
impl From<&String> for Value {
    fn from(v: &String) -> Self {
        Value::Text(v.clone())
    }
}
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Blob(v)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

/// Builds a `Vec<Value>` of statement arguments: `params![a, &b, c]`.
#[macro_export]
macro_rules! params {
    ($($v:expr),* $(,)?) => { vec![$($crate::Value::from($v)),*] };
}

const BLOB_KEY: &str = "base64";

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Value::Null => s.serialize_none(),
            Value::Integer(i) => s.serialize_i64(*i),
            Value::Real(f) => s.serialize_f64(*f),
            Value::Text(t) => s.serialize_str(t),
            Value::Blob(b) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry(
                    BLOB_KEY,
                    &base64::engine::general_purpose::STANDARD.encode(b),
                )?;
                m.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Value;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("null, a number, a string or {\"base64\": \"...\"}")
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_none<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Integer(v.into()))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Integer(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
                i64::try_from(v)
                    .map(Value::Integer)
                    .map_err(|_| E::custom("integer out of range"))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
                Ok(Value::Real(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(Value::Text(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::Text(v))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
                let Some(key) = map.next_key::<String>()? else {
                    return Err(de::Error::custom("expected {\"base64\": \"...\"}"));
                };
                if key != BLOB_KEY {
                    return Err(de::Error::unknown_field(&key, &[BLOB_KEY]));
                }
                let text: String = map.next_value()?;
                if map.next_key::<String>()?.is_some() {
                    return Err(de::Error::custom("a blob has only the \"base64\" key"));
                }
                base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .map(Value::Blob)
                    .map_err(de::Error::custom)
            }
        }
        d.deserialize_any(V)
    }
}

// ---- requests ----

/// One SQL statement and its arguments (`?`, `?1`, ... in the text).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Statement {
    pub sql: Cow<'static, str>,
    #[serde(default)]
    pub args: Vec<Value>,
}

/// Statements run in order in one transaction.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    pub statements: Vec<Statement>,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a statement; returns its index in the results. In code the SQL is always a
    /// literal; values go in `args`, never into the text.
    pub fn push(&mut self, sql: &'static str, args: Vec<Value>) -> usize {
        self.statements.push(Statement {
            sql: Cow::Borrowed(sql),
            args,
        });
        self.statements.len() - 1
    }
}

// ---- responses ----

/// What one statement did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StmtResult {
    /// Column names of `rows` (empty when there are no rows).
    #[serde(default)]
    pub columns: Vec<String>,
    /// Rows from a SELECT or a RETURNING clause.
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
    /// Rows changed by an INSERT/UPDATE/DELETE (not counting foreign key cascades).
    #[serde(default)]
    pub rows_affected: u64,
    #[serde(default)]
    pub last_insert_id: i64,
}

impl StmtResult {
    /// The first column of the first row, if there is one.
    pub fn first(&self) -> Option<&Value> {
        self.rows.first().and_then(|r| r.first())
    }

    /// Takes the first column of the first row, if there is one.
    pub fn into_first(self) -> Option<Value> {
        self.rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BatchResult {
    pub results: Vec<StmtResult>,
}

/// What kind of failure, so callers can react (e.g. "that name is taken") without parsing
/// messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    UniqueViolation,
    ForeignKeyViolation,
    NotNullViolation,
    CheckViolation,
    /// Any other error from SQLite (bad SQL, wrong argument count, ...).
    Sql,
    /// The request itself is malformed.
    BadRequest,
    Unauthorized,
    /// The service failed (I/O, pool, ...); worth retrying.
    Internal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    /// In a batch, the index of the statement that failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement: Option<usize>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.statement {
            Some(i) => write!(f, "{:?} in statement {i}: {}", self.kind, self.message),
            None => write!(f, "{:?}: {}", self.kind, self.message),
        }
    }
}

impl std::error::Error for Error {}

/// The body of every failed response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_round_trip_through_json() {
        let values = vec![
            Value::Null,
            Value::Integer(i64::MAX),
            Value::Integer(-3),
            Value::Real(2.0),
            Value::Real(-0.5),
            Value::Text("hi".into()),
            Value::Blob(vec![0, 255, 7]),
        ];
        let j = serde_json::to_value(&values).unwrap();
        assert_eq!(
            j,
            json!([null, i64::MAX, -3, 2.0, -0.5, "hi", {"base64": "AP8H"}])
        );
        assert_eq!(serde_json::to_string(&Value::Real(2.0)).unwrap(), "2.0");
        let back: Vec<Value> = serde_json::from_value(j).unwrap();
        assert_eq!(back, values);
    }

    #[test]
    fn odd_values_are_refused_or_mapped() {
        assert_eq!(
            serde_json::from_value::<Value>(json!(true)).unwrap(),
            Value::Integer(1)
        );
        assert!(serde_json::from_value::<Value>(json!(u64::MAX)).is_err());
        assert!(serde_json::from_value::<Value>(json!({"hex": "00"})).is_err());
        assert!(serde_json::from_value::<Value>(json!({"base64": "AA==", "x": 1})).is_err());
        assert!(serde_json::from_value::<Value>(json!([1])).is_err());
    }

    #[test]
    fn requests_have_a_plain_shape() {
        let mut b = Batch::new();
        b.push("SELECT ?", params![1, "a", None::<i64>]);
        assert_eq!(
            serde_json::to_value(&b).unwrap(),
            json!({"statements": [{"sql": "SELECT ?", "args": [1, "a", null]}]})
        );
        let s: Statement = serde_json::from_value(json!({"sql": "SELECT 1"})).unwrap();
        assert!(s.args.is_empty());
    }
}
