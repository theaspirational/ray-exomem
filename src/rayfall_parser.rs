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
    InExom,
    /// Anything else — pass through to ray_eval_str
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatomRole {
    Entity,
    Attribute,
    Value,
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
        "in-exom" => FormKind::InExom,
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

/// Rewrite `(query ...)` to attach `(rules ...)` structurally via the Rayfall AST.
pub fn rewrite_query_with_rules(
    form_source: &str,
    rule_inline_bodies: &[String],
) -> Result<String> {
    let expr = crate::rayfall_ast::parse_one(form_source)?;
    let lowered = crate::rayfall_ast::lower_top_level(
        &expr,
        crate::rayfall_ast::LoweringOptions {
            default_query_exom: None,
            default_rule_exom: None,
        },
    )?;
    let [crate::rayfall_ast::CanonicalForm::Query(query)] = lowered.as_slice() else {
        bail!("expected exactly one lowered query form");
    };
    crate::rayfall_ast::append_rules_to_query(query, rule_inline_bodies)
}

pub fn datom_query_projection_roles(query_source: &str) -> Option<Vec<Option<DatomRole>>> {
    let find_body = extract_clause_body(query_source, "find")?;
    let find_vars: Vec<String> = tokenize_simple(find_body)
        .into_iter()
        .filter(|t| t.starts_with('?'))
        .collect();
    if find_vars.is_empty() {
        return None;
    }

    let where_body = extract_clause_body(query_source, "where")?;
    let atoms = top_level_lists(where_body);
    if atoms.len() != 1 {
        return None;
    }
    let atom = atoms[0].trim();
    let atom_inner = atom.strip_prefix('(')?.strip_suffix(')')?.trim();
    let terms = tokenize_simple(atom_inner);
    if terms.len() != 3 {
        return None;
    }

    // Only infer datom roles for raw triple patterns. In Rayfall / rayforce2
    // a rule invocation starts with a bare predicate name, while a triple
    // pattern starts with either a ?variable, a quoted symbol / string
    // (e.g. `'foo`, `"bar"`), an underscore, or a bare integer. A rule
    // invocation like `(pair ?w ?h)` must not be interpreted as
    // `(?entity 'attr ?value)` — that misleads the i64 cell decoder into
    // calling sym_lookup on raw values.
    let first = terms.first()?.as_str();
    let looks_triple = first.starts_with('?')
        || first.starts_with('\'')
        || first.starts_with('"')
        || first == "_"
        || first.parse::<i64>().is_ok();
    if !looks_triple {
        return None;
    }

    let mut out = vec![None; find_vars.len()];
    for (i, var) in find_vars.iter().enumerate() {
        if terms[0] == *var {
            out[i] = Some(DatomRole::Entity);
        } else if terms[1] == *var {
            out[i] = Some(DatomRole::Attribute);
        } else if terms[2] == *var {
            out[i] = Some(DatomRole::Value);
        }
    }

    if out.iter().any(Option::is_some) {
        Some(out)
    } else {
        None
    }
}

fn extract_clause_body<'a>(source: &'a str, keyword: &str) -> Option<&'a str> {
    let pattern = format!("({keyword}");
    let start = source.find(&pattern)?;
    let bytes = source.as_bytes();
    let mut i = start + pattern.len();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let body_start = i;
    let mut depth = 1i32;
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
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return source.get(body_start..i).map(str::trim);
            }
        }
        i += 1;
    }
    None
}

fn top_level_lists(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] != b'(' {
            return Vec::new();
        }
        let start = i;
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
            } else if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    if let Some(slice) = body.get(start..i) {
                        out.push(slice);
                    }
                    break;
                }
                continue;
            }
            i += 1;
        }
    }
    out
}

fn tokenize_simple(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let bytes = source.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    out.push(source[start..i].to_string());
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b')' {
            i += 1;
        }
        out.push(source[start..i].to_string());
    }
    out
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

    #[test]
    fn split_in_exom_form() {
        let s = "(in-exom main (query (find ?x) (where (p ?x))))";
        let f = split_forms(s);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FormKind::InExom);
    }
}
