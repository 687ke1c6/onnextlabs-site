//! Turning result rows into Rust values with serde.
//!
//! A row deserializes into a struct (fields matched to columns by name; extra columns are
//! ignored), a tuple (columns in order), or anything else serde can build from a map or a
//! sequence. A single value deserializes into numbers, strings, `Option`s (NULL → `None`),
//! `Vec<u8>` (BLOB), unit enum variants (TEXT), and `bool` (SQLite stores 0/1).

use std::fmt;

use serde::de::{
    self, value::MapDeserializer, value::SeqDeserializer, DeserializeOwned, IntoDeserializer,
    Visitor,
};
use serde::forward_to_deserialize_any;

use crate::{Error, ErrorKind, Value};

/// A row as `T`.
pub fn from_row<T: DeserializeOwned>(columns: &[String], row: Vec<Value>) -> Result<T, Error> {
    T::deserialize(RowDeserializer { columns, row }).map_err(Into::into)
}

/// One value as `T`.
pub fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, Error> {
    T::deserialize(value).map_err(Into::into)
}

#[derive(Debug)]
pub struct DeError(String);

impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        DeError(msg.to_string())
    }
}

impl From<DeError> for Error {
    fn from(e: DeError) -> Self {
        Error {
            kind: ErrorKind::Decode,
            message: e.0,
            statement: None,
            code: None,
        }
    }
}

struct RowDeserializer<'a> {
    columns: &'a [String],
    row: Vec<Value>,
}

impl<'de> de::Deserializer<'de> for RowDeserializer<'_> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        if self.columns.len() != self.row.len() {
            return Err(DeError(format!(
                "{} columns but {} values",
                self.columns.len(),
                self.row.len()
            )));
        }
        let pairs = self.columns.iter().map(String::as_str).zip(self.row);
        let mut map = MapDeserializer::new(pairs);
        let out = visitor.visit_map(&mut map)?;
        map.end()?;
        Ok(out)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        let mut seq = SeqDeserializer::new(self.row.into_iter());
        let out = visitor.visit_seq(&mut seq)?;
        seq.end()?;
        Ok(out)
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_seq(visitor)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct map struct enum identifier ignored_any
    }
}

impl<'de> IntoDeserializer<'de, DeError> for Value {
    type Deserializer = Value;

    fn into_deserializer(self) -> Value {
        self
    }
}

impl Value {
    fn invalid(&self, expected: &dyn de::Expected) -> DeError {
        let unexpected = match self {
            Value::Null => de::Unexpected::Option,
            Value::Integer(i) => de::Unexpected::Signed(*i),
            Value::Real(f) => de::Unexpected::Float(*f),
            Value::Text(s) => de::Unexpected::Str(s),
            Value::Blob(b) => de::Unexpected::Bytes(b),
        };
        de::Error::invalid_type(unexpected, expected)
    }
}

impl<'de> de::Deserializer<'de> for Value {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self {
            Value::Null => visitor.visit_none(),
            Value::Integer(i) => visitor.visit_i64(i),
            Value::Real(f) => visitor.visit_f64(f),
            Value::Text(s) => visitor.visit_string(s),
            Value::Blob(b) => visitor.visit_byte_buf(b),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self {
            Value::Integer(i) => visitor.visit_bool(i != 0),
            v => Err(v.invalid(&visitor)),
        }
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_f64(visitor)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self {
            // e.g. sum() over whole numbers in a REAL-typed expression.
            Value::Integer(i) => visitor.visit_f64(i as f64),
            v => v.deserialize_any(visitor),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self {
            Value::Null => visitor.visit_none(),
            v => visitor.visit_some(v),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self {
            Value::Null => visitor.visit_unit(),
            v => Err(v.invalid(&visitor)),
        }
    }

    /// `Vec<u8>` asks for a sequence.
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self {
            Value::Blob(b) => {
                let mut seq = SeqDeserializer::new(b.into_iter());
                let out = visitor.visit_seq(&mut seq)?;
                seq.end()?;
                Ok(out)
            }
            v => Err(v.invalid(&visitor)),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_newtype_struct(self)
    }

    /// Unit variants stored as their name.
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match self {
            Value::Text(s) => visitor.visit_enum(s.into_deserializer()),
            v => Err(v.invalid(&visitor)),
        }
    }

    forward_to_deserialize_any! {
        i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 char str string bytes byte_buf
        unit_struct tuple tuple_struct map struct identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[derive(Deserialize, Debug, PartialEq)]
    #[allow(dead_code)]
    enum Kind {
        Cascade,
        Restrict,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Thing {
        id: i64,
        #[serde(rename = "type")]
        category: Option<String>,
        manual: bool,
        amount: f64,
        hash: Vec<u8>,
        kind: Kind,
        missing: Option<i32>,
        #[serde(skip)]
        computed: u8,
    }

    #[test]
    fn rows_become_structs_by_column_name() {
        let t: Thing = from_row(
            &cols(&["id", "type", "manual", "amount", "hash", "kind", "extra"]),
            vec![
                Value::Integer(3),
                Value::Null,
                Value::Integer(1),
                Value::Integer(10),
                Value::Blob(vec![9]),
                Value::Text("Cascade".into()),
                Value::Text("ignored".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            t,
            Thing {
                id: 3,
                category: None,
                manual: true,
                amount: 10.0,
                hash: vec![9],
                kind: Kind::Cascade,
                missing: None,
                computed: 0,
            }
        );
    }

    #[test]
    fn rows_become_tuples_in_order_and_values_become_scalars() {
        let t: (i64, String, Option<bool>) = from_row(
            &cols(&["a", "b", "c"]),
            vec![Value::Integer(1), Value::Text("x".into()), Value::Null],
        )
        .unwrap();
        assert_eq!(t, (1, "x".into(), None));
        assert_eq!(from_value::<i32>(Value::Integer(7)).unwrap(), 7);
        assert_eq!(from_value::<Option<String>>(Value::Null).unwrap(), None);
    }

    #[test]
    fn mismatches_are_decode_errors() {
        let e = from_value::<i64>(Value::Text("x".into())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Decode);
        assert!(from_value::<u8>(Value::Integer(300)).is_err());
        assert!(from_value::<String>(Value::Null).is_err());
        let e = from_row::<(i64,)>(&cols(&["a"]), vec![]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Decode);
    }
}
