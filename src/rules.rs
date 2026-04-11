//! Rule parsing and derived predicate extraction.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};

use crate::context::MutationContext;

/// A parsed rule: head predicate name + arity + inline body for `(rules ...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRule {
    pub head_predicate: String,
    pub head_arity: usize,
    /// Full `(rule ...)` line for persistence and export.
    pub full_text: String,
    /// Fragment for `(rules R1 R2 ...)` — each `R` is `((head) (body)... )`.
    pub inline_body: String,
    pub defined_by: MutationContext,
    pub defined_at: String,
}

/// Scan a balanced `( ... )` starting at the first `(` in `input`.
fn scan_balanced_from_open(input: &str) -> Option<&str> {
    let start = input.find('(')?;
    let mut depth = 0usize;
    for (i, c) in input[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + i + 1;
                    return Some(&input[start..end]);
                }
            }
            _ => {}
        }
    }
    None
}

fn skip_ws(s: &str) -> &str {
    s.trim_start()
}

/// Extract `((head) clauses...)` for inline rules from a full `(rule ...)` line.
pub fn extract_inline_rule_body(full_rule: &str) -> Result<String> {
    let t = full_rule.trim();
    let after = t
        .strip_prefix("(rule")
        .ok_or_else(|| anyhow!("expected (rule"))?;
    let mut s = skip_ws(after);
    if !s.starts_with('(') {
        let end = s
            .find(|c: char| c.is_whitespace() || c == '(')
            .ok_or_else(|| anyhow!("malformed rule after (rule"))?;
        s = skip_ws(&s[end..]);
    }
    let mut parts: Vec<String> = Vec::new();
    loop {
        s = skip_ws(s);
        if s.is_empty() || s.starts_with(')') {
            break;
        }
        let sub = scan_balanced_from_open(s).ok_or_else(|| anyhow!("unbalanced rule body"))?;
        let off = s.find(sub).unwrap_or(0);
        parts.push(sub.to_string());
        s = &s[off + sub.len()..];
    }
    if parts.is_empty() {
        bail!("empty rule body");
    }
    Ok(format!("({})", parts.join(" ")))
}

/// Parse head predicate and arity from an inline rule body `((pred ?a ?b) (clause)...)`.
pub fn parse_rule_head(inline_body: &str) -> Result<(String, usize)> {
    let trimmed = inline_body.trim();
    let head_source = if let Some(rest) = trimmed.strip_prefix('(') {
        rest.trim_start()
    } else {
        trimmed
    };
    let head =
        scan_balanced_from_open(head_source).ok_or_else(|| anyhow!("could not find rule head"))?;
    let head_inner = head
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .ok_or_else(|| anyhow!("bad head"))?;
    let mut parts = head_inner.split_whitespace();
    let pred = parts
        .next()
        .ok_or_else(|| anyhow!("empty rule head"))?
        .trim();
    if pred.is_empty() {
        bail!("empty predicate in rule head");
    }
    let arity = head_inner.matches('?').count();
    Ok((pred.to_string(), arity))
}

/// Build a [ParsedRule] from a stored or evaluated `(rule ...)` line.
pub fn parse_rule_line(
    line: &str,
    defined_by: MutationContext,
    defined_at: String,
) -> Result<ParsedRule> {
    let full_text = line.trim().to_string();
    if full_text.is_empty() {
        bail!("empty rule line");
    }
    let inline_body = extract_inline_rule_body(&full_text)?;
    let (head_predicate, head_arity) = parse_rule_head(&inline_body)?;
    Ok(ParsedRule {
        head_predicate,
        head_arity,
        full_text,
        inline_body,
        defined_by,
        defined_at,
    })
}

/// Collect unique derived predicates from a rule set (max arity per name).
pub fn derived_predicates(rules: &[ParsedRule]) -> Vec<(String, usize)> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for r in rules {
        seen.entry(r.head_predicate.clone())
            .and_modify(|a: &mut usize| *a = (*a).max(r.head_arity))
            .or_insert(r.head_arity);
    }
    seen.into_iter().collect()
}
