//! Typed values for facts.
//!
//! `FactValue` carries type info end-to-end (API → assert → JSONL → splay →
//! datalog query). This enables comparison / aggregation operators in Rayfall
//! rules over numeric fact values without re-parsing strings.
//!
//! ## Serde encoding (JSON)
//!
//! `#[serde(untagged)]` picks the variant from the JSON shape:
//!   * `20` (number) → `FactValue::I64`
//!   * `{"$sym": "foo"}` (object) → `FactValue::Sym`
//!   * `"abc"` (string) → `FactValue::Str` (fallback)
//!
//! Variant ORDER is load-bearing — I64 first, Sym next, Str last. Existing
//! JSONL files that wrote `"75"` (string) keep loading as `FactValue::Str("75")`
//! until a typed assert replaces them.
//!
//! ## Splay encoding
//!
//! The rayforce2 splay tables speak tagged int64 datoms:
//!   * I64 → the raw i64 (no tag bits; bit 63 reserved for negatives)
//!   * Sym → `encode_sym(intern)` (bits 63-62 = 01)
//!   * Str → `encode_str(intern)` (bits 63-62 = 10)
//!
//! See `src/datom.rs` for the full tagging scheme.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Typed fact value used in `Fact.value` and the `assert-fact` API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FactValue {
    /// Plain integer — matches Rayfall bare int literals in queries and enables
    /// `<` / `>` / `sum` / `avg` operators against stored values.
    I64(i64),
    /// Symbol — interned, identity-compared. Surfaces as a JSON object
    /// (`{"$sym": "..."}`). Reserved for explicit opt-in through the API or
    /// the CLI `--as-sym` flag.
    Sym(SymValue),
    /// Default UTF-8 string. Matches Rayfall string literals in rule bodies.
    Str(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymValue {
    #[serde(rename = "$sym")]
    pub sym: String,
}

impl FactValue {
    /// Construct a `FactValue::Sym` from anything string-like.
    pub fn sym(s: impl Into<String>) -> Self {
        FactValue::Sym(SymValue { sym: s.into() })
    }

    /// Auto-detect from a raw CLI / text input:
    ///   * parses as `i64` with a round-trip check → `I64`
    ///   * otherwise `Str`
    ///
    /// The round-trip check rejects values where `n.to_string() != input`,
    /// which preserves strings like `"007"` (leading zeros), `"+5"` (explicit
    /// sign), and `"7.5"` (float). Callers that want strict `i64` parsing
    /// should use `FactValue::I64(n)` directly.
    pub fn auto(s: &str) -> Self {
        if let Ok(n) = s.parse::<i64>() {
            if n.to_string() == s {
                return FactValue::I64(n);
            }
        }
        FactValue::Str(s.to_string())
    }

    /// Display form used in logs, tx summaries, Rayfall rule templates,
    /// and any legacy context that previously held a `String` value.
    pub fn display(&self) -> String {
        match self {
            FactValue::I64(n) => n.to_string(),
            FactValue::Sym(s) => format!("'{}", s.sym),
            FactValue::Str(s) => s.clone(),
        }
    }

    /// Short name of the variant — useful for API metadata (`value_kind`) and
    /// for the splay encoder to decide which tag to use.
    pub fn kind(&self) -> &'static str {
        match self {
            FactValue::I64(_) => "i64",
            FactValue::Sym(_) => "sym",
            FactValue::Str(_) => "str",
        }
    }

    /// Borrow as str when the variant is `Str`. Returns `None` for `I64` / `Sym`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FactValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Extract i64 when the variant is `I64`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            FactValue::I64(n) => Some(*n),
            _ => None,
        }
    }

    /// Borrow sym name when the variant is `Sym`.
    pub fn as_sym(&self) -> Option<&str> {
        match self {
            FactValue::Sym(s) => Some(s.sym.as_str()),
            _ => None,
        }
    }
}

impl fmt::Display for FactValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FactValue::I64(n) => fmt::Display::fmt(n, f),
            FactValue::Sym(s) => write!(f, "'{}", s.sym),
            FactValue::Str(s) => f.write_str(s),
        }
    }
}

impl Default for FactValue {
    fn default() -> Self {
        FactValue::Str(String::new())
    }
}

// --- Conversions --------------------------------------------------------------

impl From<&str> for FactValue {
    fn from(s: &str) -> Self {
        FactValue::Str(s.to_string())
    }
}

impl From<String> for FactValue {
    fn from(s: String) -> Self {
        FactValue::Str(s)
    }
}

impl From<&String> for FactValue {
    fn from(s: &String) -> Self {
        FactValue::Str(s.clone())
    }
}

impl From<i64> for FactValue {
    fn from(n: i64) -> Self {
        FactValue::I64(n)
    }
}

impl From<&FactValue> for FactValue {
    fn from(v: &FactValue) -> Self {
        v.clone()
    }
}

// --- Ergonomic equality against bare strings (used widely in tests) ----------

impl PartialEq<str> for FactValue {
    fn eq(&self, other: &str) -> bool {
        match self {
            FactValue::Str(s) => s == other,
            FactValue::I64(n) => n.to_string() == other,
            FactValue::Sym(s) => s.sym == other,
        }
    }
}

impl PartialEq<&str> for FactValue {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl PartialEq<String> for FactValue {
    fn eq(&self, other: &String) -> bool {
        self == other.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_i64() {
        assert_eq!(FactValue::auto("75"), FactValue::I64(75));
        assert_eq!(FactValue::auto("0"), FactValue::I64(0));
        assert_eq!(FactValue::auto("-17"), FactValue::I64(-17));
    }

    #[test]
    fn auto_str_for_non_round_trip_numbers() {
        // leading zeros — "007".parse::<i64>() = Ok(7) but "7" != "007"
        assert_eq!(
            FactValue::auto("007"),
            FactValue::Str("007".to_string())
        );
        // explicit plus sign — "+5".parse::<i64>() = Ok(5) but "5" != "+5"
        assert_eq!(
            FactValue::auto("+5"),
            FactValue::Str("+5".to_string())
        );
        // floats don't parse as i64 at all
        assert_eq!(
            FactValue::auto("7.5"),
            FactValue::Str("7.5".to_string())
        );
    }

    #[test]
    fn auto_str_for_words() {
        assert_eq!(
            FactValue::auto("active"),
            FactValue::Str("active".to_string())
        );
        assert_eq!(
            FactValue::auto(""),
            FactValue::Str(String::new())
        );
    }

    #[test]
    fn display_forms() {
        assert_eq!(FactValue::I64(42).display(), "42");
        assert_eq!(FactValue::Str("hello".into()).display(), "hello");
        assert_eq!(FactValue::sym("active").display(), "'active");
    }

    #[test]
    fn from_impls() {
        let _a: FactValue = "hi".into();
        let _b: FactValue = String::from("hi").into();
        let _c: FactValue = 42i64.into();
        assert_eq!(FactValue::from("x"), FactValue::Str("x".to_string()));
        assert_eq!(FactValue::from(7i64), FactValue::I64(7));
    }

    #[test]
    fn serde_untagged_i64() {
        let v: FactValue = serde_json::from_str("20").unwrap();
        assert_eq!(v, FactValue::I64(20));
        assert_eq!(serde_json::to_string(&v).unwrap(), "20");
    }

    #[test]
    fn serde_untagged_str() {
        let v: FactValue = serde_json::from_str("\"Basil\"").unwrap();
        assert_eq!(v, FactValue::Str("Basil".to_string()));
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"Basil\"");
    }

    #[test]
    fn serde_untagged_sym() {
        let v: FactValue = serde_json::from_str(r#"{"$sym":"active"}"#).unwrap();
        assert_eq!(v, FactValue::sym("active"));
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"$sym":"active"}"#
        );
    }

    #[test]
    fn jsonl_roundtrip_preserves_variant() {
        for v in [
            FactValue::I64(75),
            FactValue::Str("Basil".into()),
            FactValue::sym("green"),
        ] {
            let text = serde_json::to_string(&v).unwrap();
            let round: FactValue = serde_json::from_str(&text).unwrap();
            assert_eq!(round, v, "round-trip lost variant for {text}");
        }
    }

    #[test]
    fn partial_eq_with_str() {
        assert_eq!(FactValue::Str("blue".into()), "blue");
        assert_eq!(FactValue::I64(7), "7");
        assert_eq!(FactValue::sym("active"), "active");
        assert_ne!(FactValue::Str("blue".into()), "red");
    }
}
