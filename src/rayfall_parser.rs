//! Lightweight Rayfall top-level form splitter.
//!
//! Splits a Rayfall source string into top-level forms by balancing parens.
//! Does NOT parse nested structure — just identifies boundaries.

use anyhow::{bail, Result};

/// A top-level form with its kind identified by the first symbol.
pub struct TopLevelForm {
    pub kind: FormKind,
    pub source: String,
    /// Text after the opening `(symbol` (leading symbol and space skipped).
    pub inner_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormKind {
    AssertFact,
    RetractFact,
    Rule,
    Query,
    /// Anything else — pass through to ray_eval_str
    Other,
}

/// Split source into top-level forms.
pub fn split_forms(source: &str) -> Vec<TopLevelForm> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b';' && bytes.get(i + 1) == Some(&b';') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] != b'(' {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let atom = source.get(start..i).unwrap_or("").to_string();
            out.push(TopLevelForm {
                kind: FormKind::Other,
                source: atom,
                inner_source: String::new(),
            });
            continue;
        }
        let form_start = i;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape = false;
        while i < bytes.len() {
            let b = bytes[i];
            if in_string {
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    in_string = false;
                }
                i += 1;
                continue;
            }
            if b == b'"' {
                in_string = true;
                i += 1;
                continue;
            }
            if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    break;
                }
                continue;
            }
            i += 1;
        }
        let form_end = i.min(bytes.len());
        let slice = source.get(form_start..form_end).unwrap_or("").to_string();
        let kind = classify_form(&slice);
        let inner = inner_after_first_symbol(&slice);
        out.push(TopLevelForm {
            kind,
            source: slice,
            inner_source: inner,
        });
    }
    out
}

fn classify_form(form: &str) -> FormKind {
    let s = form.trim();
    let s = s.strip_prefix('(').unwrap_or(s).trim_start();
    let sym = s.split_whitespace().next().unwrap_or("");
    match sym {
        "assert-fact" => FormKind::AssertFact,
        "retract-fact" => FormKind::RetractFact,
        "rule" => FormKind::Rule,
        "query" => FormKind::Query,
        _ => FormKind::Other,
    }
}

fn inner_after_first_symbol(form: &str) -> String {
    let trimmed = form.trim();
    let Some(rest) = trimmed.strip_prefix('(') else {
        return String::new();
    };
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ')')
        .unwrap_or(rest.len());
    rest[end..].trim().to_string()
}

/// Parse `(assert-fact exom entity pred value)` / `(retract-fact ...)` args.
/// Entity and value are string datoms; predicate is a symbol after `'`.
pub fn parse_fact_mutation_args(inner: &str) -> Result<(String, String, String, String)> {
    let mut lexer = Lexer::new(inner);
    let exom = lexer.next_token()?;
    let entity = lexer.next_string()?;
    let predicate = lexer.next_quoted_symbol()?;
    let value = lexer.next_string()?;
    Ok((exom, entity, predicate, value))
}

struct Lexer<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Self { s: s.trim(), i: 0 }
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s.as_bytes()[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn next_token(&mut self) -> Result<String> {
        self.skip_ws();
        if self.i >= self.s.len() {
            bail!("unexpected end of input (expected token)");
        }
        let b = self.s.as_bytes()[self.i];
        if b == b'"' {
            return self.next_string();
        }
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s.as_bytes()[self.i];
            if c.is_ascii_whitespace() || c == b')' {
                break;
            }
            self.i += 1;
        }
        Ok(self.s[start..self.i].to_string())
    }

    fn next_string(&mut self) -> Result<String> {
        self.skip_ws();
        if self.i >= self.s.len() || self.s.as_bytes()[self.i] != b'"' {
            bail!("expected string literal");
        }
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s.as_bytes()[self.i];
            if c == b'\\' {
                self.i += 2;
                continue;
            }
            if c == b'"' {
                let raw = &self.s[start..self.i];
                self.i += 1;
                return Ok(unescape(raw));
            }
            self.i += 1;
        }
        bail!("unterminated string");
    }

    fn next_quoted_symbol(&mut self) -> Result<String> {
        self.skip_ws();
        if self.i >= self.s.len() {
            bail!("expected quoted symbol");
        }
        if self.s.as_bytes()[self.i] == b'\'' {
            self.i += 1;
        }
        self.next_token()
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'"') {
            chars.next();
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out
}

/// Rewrite `(query ...)` to insert `(rules ...)` before the closing `)` of the outer form.
pub fn rewrite_query_with_rules(form_source: &str, rule_inline_bodies: &[String]) -> String {
    if rule_inline_bodies.is_empty() {
        return form_source.to_string();
    }
    let trimmed = form_source.trim_end();
    let Some(without_close) = trimmed.strip_suffix(')') else {
        return form_source.to_string();
    };
    let mut result = without_close.to_string();
    result.push_str(" (rules");
    for rule in rule_inline_bodies {
        result.push(' ');
        result.push_str(rule);
    }
    result.push_str("))");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_two_forms() {
        let s = "(+ 1 2) (+ 3 4)";
        let f = split_forms(s);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].kind, FormKind::Other);
    }

    #[test]
    fn assert_fact_parse() {
        let inner = r#"main "e" 'p "v""#;
        let (exom, e, p, v) = parse_fact_mutation_args(inner).unwrap();
        assert_eq!(exom, "main");
        assert_eq!(e, "e");
        assert_eq!(p, "p");
        assert_eq!(v, "v");
    }
}
