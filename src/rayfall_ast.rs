use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    List(Vec<Expr>),
    Symbol(String),
    String(String),
    Quote(Box<Expr>),
}

impl Expr {
    pub fn list(items: Vec<Expr>) -> Self {
        Self::List(items)
    }

    pub fn symbol(value: impl Into<String>) -> Self {
        Self::Symbol(value.into())
    }

    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    pub fn as_list(&self) -> Option<&[Expr]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_symbol(&self) -> Option<&str> {
        match self {
            Self::Symbol(value) => Some(value),
            _ => None,
        }
    }

    pub fn emit(&self) -> String {
        match self {
            Self::List(items) => {
                let rendered = items.iter().map(Self::emit).collect::<Vec<_>>().join(" ");
                format!("({rendered})")
            }
            Self::Symbol(value) => value.clone(),
            Self::String(value) => format!("\"{}\"", escape_string(value)),
            Self::Quote(inner) => format!("'{}", inner.emit()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoweringOptions<'a> {
    pub default_query_exom: Option<&'a str>,
    pub default_rule_exom: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalQuery {
    pub exom: String,
    pub clauses: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRule {
    pub exom: String,
    pub clauses: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFactMutation {
    pub exom: String,
    pub fact_id: String,
    pub predicate: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFactRetract {
    pub exom: String,
    pub fact_id: String,
    /// `(predicate, value)` the fact must currently hold. `None` retracts
    /// whatever it holds — the only way to retract a typed (number or symbol)
    /// value, which a Rayfall string literal cannot name.
    pub tuple: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalForm {
    Query(CanonicalQuery),
    Rule(CanonicalRule),
    AssertFact(CanonicalFactMutation),
    RetractFact(CanonicalFactRetract),
}

impl CanonicalQuery {
    pub fn to_expr(&self) -> Expr {
        let mut items = vec![Expr::symbol("query"), Expr::symbol(self.exom.clone())];
        items.extend(self.clauses.clone());
        Expr::list(items)
    }

    pub fn emit(&self) -> String {
        self.to_expr().emit()
    }

    /// Pin body-atom string literals to sym tags when the schema demands it.
    ///
    /// rayforce's column-aware compare matches body literals against typed
    /// columns by tag. Predicate names and entity refs are stored as
    /// SYM-tagged datoms, but the surface syntax `(?e 'fact/predicate "p")`
    /// emits the value as a STR-tagged datom — tag mismatch → 0 rows. For
    /// every body atom shaped `(?e 'attr literal)` whose attribute resolves
    /// in the schema to a sym-encoded `value_kind`, rewrite the literal to
    /// `'literal` so the engine sym-interns it before the compare.
    pub fn rewrite_body_literals_with_schema<F>(&mut self, value_kind_for_attr: F)
    where
        F: Fn(&str) -> Option<String>,
    {
        self.rewrite_body_literals_with_schema_and_rules(value_kind_for_attr, |_| None);
    }

    /// Like `rewrite_body_literals_with_schema`, but also rewrites literal
    /// args of rule-call body atoms.
    ///
    /// Rule expansion happens server-side in rayforce, so a literal in a
    /// rule-call slot — e.g. `"test/n"` in `(fact-row ?id "test/n" ?v)` —
    /// only lands in the inlined `(?fact 'fact/predicate "test/n")` AFTER
    /// the query string leaves Rust. Rewriting at expansion-time on our
    /// side won't reach it. Instead, derive a per-head-param attribute map
    /// from each rule definition (`derive_rule_param_attrs`) and pin the
    /// rule-call literals here, before the engine ever sees them.
    pub fn rewrite_body_literals_with_schema_and_rules<F, G>(
        &mut self,
        value_kind_for_attr: F,
        rule_param_attrs: G,
    ) where
        F: Fn(&str) -> Option<String>,
        G: Fn(&str) -> Option<Vec<Option<String>>>,
    {
        for clause in &mut self.clauses {
            rewrite_expr_body_literals(clause, &value_kind_for_attr, &rule_param_attrs);
        }
    }
}

const SYM_ENCODED_KINDS: &[&str] = &["predicate", "branch", "tx-entity", "entity", "sym"];

fn pin_string_to_sym<F>(arg: &mut Expr, attr: &str, value_kind_for_attr: &F)
where
    F: Fn(&str) -> Option<String>,
{
    if let Expr::String(s) = arg {
        if let Some(kind) = value_kind_for_attr(attr) {
            if SYM_ENCODED_KINDS.contains(&kind.as_str()) {
                let value = s.clone();
                *arg = Expr::Quote(Box::new(Expr::Symbol(value)));
            }
        }
    }
}

fn rewrite_expr_body_literals<F, G>(expr: &mut Expr, value_kind_for_attr: &F, rule_param_attrs: &G)
where
    F: Fn(&str) -> Option<String>,
    G: Fn(&str) -> Option<Vec<Option<String>>>,
{
    if let Expr::List(items) = expr {
        // EAV body atom: `(?ent 'attr "literal")` → `(?ent 'attr 'literal)`.
        if items.len() == 3 {
            let attr_name = match &items[1] {
                Expr::Quote(inner) => match inner.as_ref() {
                    Expr::Symbol(s) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(attr) = attr_name {
                pin_string_to_sym(&mut items[2], &attr, value_kind_for_attr);
            }
        }

        // Rule-call body atom: `(rule-name arg0 arg1 ... argN)`. Each argN
        // matches one head-param slot of the rule; if that slot is bound to
        // an attribute's value position in the rule body, the literal needs
        // the same sym-pin treatment.
        if let Some(name) = items.first().and_then(Expr::as_symbol) {
            if let Some(param_attrs) = rule_param_attrs(name) {
                for (i, arg) in items.iter_mut().enumerate().skip(1) {
                    let slot = i - 1;
                    if let Some(Some(attr)) = param_attrs.get(slot) {
                        pin_string_to_sym(arg, attr, value_kind_for_attr);
                    }
                }
            }
        }

        for child in items.iter_mut() {
            rewrite_expr_body_literals(child, value_kind_for_attr, rule_param_attrs);
        }
    }
}

/// Derive a per-head-param attribute mapping from a rule's inline body.
///
/// Returns `(head_predicate, per-slot attr names)`. For a rule like
/// `((fact-row ?fact ?pred ?value) (?fact 'fact/predicate ?pred) (?fact 'fact/value ?value))`,
/// returns `("fact-row", [None, Some("fact/predicate"), Some("fact/value")])` —
/// slot 0 (?fact) is an entity slot (no value-position attr), slot 1 (?pred)
/// is the value of `fact/predicate`, slot 2 (?value) is the value of
/// `fact/value`. Used by the rule-call rewriter to pin sym-encoded literals
/// before the rule expands inside the engine.
pub fn derive_rule_param_attrs(inline_body: &str) -> Option<(String, Vec<Option<String>>)> {
    use std::collections::HashMap;

    let parsed = parse_one(inline_body).ok()?;
    let outer = parsed.as_list()?;
    let head = outer.first()?.as_list()?;
    let head_pred = head.first()?.as_symbol()?.to_string();
    let head_vars: Vec<Option<String>> = head[1..]
        .iter()
        .map(|e| match e {
            Expr::Symbol(s) if s.starts_with('?') => Some(s.clone()),
            _ => None,
        })
        .collect();

    let mut value_var_to_attr: HashMap<String, String> = HashMap::new();
    for clause in &outer[1..] {
        let Some(items) = clause.as_list() else {
            continue;
        };
        if items.len() != 3 {
            continue;
        }
        let attr = match &items[1] {
            Expr::Quote(inner) => match inner.as_ref() {
                Expr::Symbol(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        };
        let value_var = match &items[2] {
            Expr::Symbol(s) if s.starts_with('?') => Some(s.clone()),
            _ => None,
        };
        if let (Some(a), Some(v)) = (attr, value_var) {
            value_var_to_attr.entry(v).or_insert(a);
        }
    }

    let param_attrs = head_vars
        .into_iter()
        .map(|v| v.and_then(|name| value_var_to_attr.get(&name).cloned()))
        .collect();

    Some((head_pred, param_attrs))
}

impl CanonicalRule {
    pub fn to_expr(&self) -> Expr {
        let mut items = vec![Expr::symbol("rule"), Expr::symbol(self.exom.clone())];
        items.extend(self.clauses.clone());
        Expr::list(items)
    }

    pub fn emit(&self) -> String {
        self.to_expr().emit()
    }
}

impl CanonicalFactMutation {
    fn to_expr_with_head(&self, head: &str) -> Expr {
        Expr::list(vec![
            Expr::symbol(head),
            Expr::symbol(self.exom.clone()),
            Expr::string(self.fact_id.clone()),
            Expr::Quote(Box::new(Expr::symbol(self.predicate.clone()))),
            Expr::string(self.value.clone()),
        ])
    }
}

impl CanonicalFactRetract {
    fn to_expr(&self) -> Expr {
        let mut items = vec![
            Expr::symbol("retract-fact"),
            Expr::symbol(self.exom.clone()),
            Expr::string(self.fact_id.clone()),
        ];
        if let Some((predicate, value)) = &self.tuple {
            items.push(Expr::Quote(Box::new(Expr::symbol(predicate.clone()))));
            items.push(Expr::string(value.clone()));
        }
        Expr::list(items)
    }
}

impl CanonicalForm {
    pub fn emit(&self) -> String {
        self.to_expr().emit()
    }

    pub fn to_expr(&self) -> Expr {
        match self {
            Self::Query(query) => query.to_expr(),
            Self::Rule(rule) => rule.to_expr(),
            Self::AssertFact(mutation) => mutation.to_expr_with_head("assert-fact"),
            Self::RetractFact(retract) => retract.to_expr(),
        }
    }
}

pub fn parse_forms(source: &str) -> Result<Vec<Expr>> {
    let mut parser = Parser::new(source);
    let mut out = Vec::new();
    loop {
        parser.skip_ws_and_comments();
        if parser.is_eof() {
            break;
        }
        out.push(parser.parse_expr()?);
    }
    Ok(out)
}

pub fn parse_one(source: &str) -> Result<Expr> {
    let forms = parse_forms(source)?;
    match forms.as_slice() {
        [form] => Ok(form.clone()),
        [] => bail!("expected exactly one Rayfall form"),
        _ => bail!("expected exactly one Rayfall form"),
    }
}

pub fn lower_top_level(expr: &Expr, options: LoweringOptions<'_>) -> Result<Vec<CanonicalForm>> {
    lower_with_env(expr, None, options, false)
}

pub fn append_rules_to_query(
    query: &CanonicalQuery,
    rule_inline_bodies: &[String],
) -> Result<String> {
    if rule_inline_bodies.is_empty() {
        return Ok(query.emit());
    }

    let mut clauses = query.clauses.clone();
    let mut rule_items = vec![Expr::symbol("rules")];
    for body in rule_inline_bodies {
        rule_items.push(parse_one(body)?);
    }
    clauses.push(Expr::list(rule_items));

    Ok(CanonicalQuery {
        exom: query.exom.clone(),
        clauses,
    }
    .emit())
}

fn lower_with_env(
    expr: &Expr,
    env_exom: Option<&str>,
    options: LoweringOptions<'_>,
    inside_in_exom: bool,
) -> Result<Vec<CanonicalForm>> {
    let items = expr
        .as_list()
        .ok_or_else(|| anyhow!("expected list form"))?;
    let head = items
        .first()
        .and_then(Expr::as_symbol)
        .ok_or_else(|| anyhow!("expected form head symbol"))?;

    match head {
        "in-exom" => {
            if inside_in_exom {
                bail!("unsupported nested form inside in-exom: in-exom");
            }
            lower_in_exom(items, options)
        }
        "query" => Ok(vec![CanonicalForm::Query(lower_query(
            items,
            env_exom,
            options.default_query_exom,
        )?)]),
        "rule" => Ok(vec![CanonicalForm::Rule(lower_rule(
            items,
            env_exom,
            options.default_rule_exom,
        )?)]),
        "assert-fact" => Ok(vec![CanonicalForm::AssertFact(lower_fact_mutation(
            "assert-fact",
            items,
            env_exom,
        )?)]),
        "retract-fact" => Ok(vec![CanonicalForm::RetractFact(lower_fact_retract(
            items, env_exom,
        )?)]),
        other if env_exom.is_some() => bail!("unsupported nested form inside in-exom: {other}"),
        other => bail!("unsupported exom-aware form: {other}"),
    }
}

fn lower_in_exom(items: &[Expr], options: LoweringOptions<'_>) -> Result<Vec<CanonicalForm>> {
    if items.len() < 3 {
        bail!("in-exom requires an exom name and at least one body form");
    }
    let exom = expect_symbol(&items[1], "in-exom exom name")?.to_string();
    let mut out = Vec::new();
    for child in &items[2..] {
        let child_head = child
            .as_list()
            .and_then(|parts| parts.first())
            .and_then(Expr::as_symbol)
            .ok_or_else(|| anyhow!("unsupported nested form inside in-exom"))?;
        match child_head {
            "query" | "rule" | "assert-fact" | "retract-fact" => {
                out.extend(lower_with_env(child, Some(exom.as_str()), options, true)?);
            }
            other => bail!("unsupported nested form inside in-exom: {other}"),
        }
    }
    Ok(out)
}

fn lower_query(
    items: &[Expr],
    env_exom: Option<&str>,
    default_exom: Option<&str>,
) -> Result<CanonicalQuery> {
    let args = &items[1..];
    let (explicit_exom, clause_start) = maybe_explicit_exom(args);
    let exom = resolve_exom(
        "query",
        explicit_exom,
        env_exom,
        default_exom,
        "query missing database name",
    )?;
    let clauses = args[clause_start..].to_vec();
    if clauses.is_empty() {
        bail!("query requires at least one clause");
    }
    Ok(CanonicalQuery { exom, clauses })
}

fn lower_rule(
    items: &[Expr],
    env_exom: Option<&str>,
    default_exom: Option<&str>,
) -> Result<CanonicalRule> {
    let args = &items[1..];
    let (explicit_exom, clause_start) = maybe_explicit_exom(args);
    let exom = resolve_exom(
        "rule",
        explicit_exom,
        env_exom,
        default_exom,
        "rule missing exom name",
    )?;
    let clauses = args[clause_start..].to_vec();
    if clauses.is_empty() {
        bail!("rule requires a head and at least one clause");
    }
    Ok(CanonicalRule { exom, clauses })
}

fn lower_fact_mutation(
    head: &str,
    items: &[Expr],
    env_exom: Option<&str>,
) -> Result<CanonicalFactMutation> {
    let args = &items[1..];
    let (explicit_exom, arg_start) = match args.len() {
        3 => (None, 0),
        4 => (
            Some(expect_symbol(&args[0], &format!("{head} exom name"))?.to_string()),
            1,
        ),
        _ => bail!("{head} requires either 3 args inside in-exom or 4 args with an explicit exom"),
    };

    let exom = resolve_exom(
        head,
        explicit_exom.as_deref(),
        env_exom,
        None,
        &format!("{head} missing exom name"),
    )?;
    let fact_id = expect_string(&args[arg_start], "fact id")?.to_string();
    let predicate = expect_predicate(&args[arg_start + 1])?.to_string();
    let value = expect_string(&args[arg_start + 2], "fact value")?.to_string();
    Ok(CanonicalFactMutation {
        exom,
        fact_id,
        predicate,
        value,
    })
}

/// `(retract-fact [exom] "id")` retracts by id; `(retract-fact [exom] "id"
/// 'pred "value")` also requires the fact to hold that tuple.
fn lower_fact_retract(items: &[Expr], env_exom: Option<&str>) -> Result<CanonicalFactRetract> {
    let head = "retract-fact";
    let args = &items[1..];
    if args.len() > 2 {
        let mutation = lower_fact_mutation(head, items, env_exom)?;
        return Ok(CanonicalFactRetract {
            exom: mutation.exom,
            fact_id: mutation.fact_id,
            tuple: Some((mutation.predicate, mutation.value)),
        });
    }
    let (explicit_exom, id) = match args {
        [id] => (None, id),
        [exom, id] => (Some(expect_symbol(exom, "retract-fact exom name")?), id),
        _ => bail!("{head} requires a fact id"),
    };
    Ok(CanonicalFactRetract {
        exom: resolve_exom(
            head,
            explicit_exom,
            env_exom,
            None,
            "retract-fact missing exom name",
        )?,
        fact_id: expect_string(id, "fact id")?.to_string(),
        tuple: None,
    })
}

fn maybe_explicit_exom(args: &[Expr]) -> (Option<&str>, usize) {
    match args.first().and_then(Expr::as_symbol) {
        Some(exom) => (Some(exom), 1),
        None => (None, 0),
    }
}

fn resolve_exom(
    _form_name: &str,
    explicit_exom: Option<&str>,
    env_exom: Option<&str>,
    default_exom: Option<&str>,
    missing_message: &str,
) -> Result<String> {
    // Exoms are keyed by slash path; `a::b::main` is the same exom spelled
    // the CLI way.
    let slash = |name: &str| name.replace("::", "/");
    let explicit_exom = explicit_exom.map(slash);
    if let Some(env_name) = env_exom.map(slash) {
        return match explicit_exom {
            Some(explicit) if explicit != env_name => bail!(
                "conflicting explicit exom '{}' inside in-exom '{}'",
                explicit,
                env_name
            ),
            _ => Ok(env_name),
        };
    }

    if let Some(explicit) = explicit_exom {
        return Ok(explicit);
    }
    if let Some(default) = default_exom {
        return Ok(slash(default));
    }

    bail!("{missing_message}");
}

fn expect_symbol<'a>(expr: &'a Expr, what: &str) -> Result<&'a str> {
    expr.as_symbol()
        .ok_or_else(|| anyhow!("expected {what} as a symbol"))
}

fn expect_string<'a>(expr: &'a Expr, what: &str) -> Result<&'a str> {
    match expr {
        Expr::String(value) => Ok(value),
        _ => bail!("expected {what} as a string"),
    }
}

fn expect_predicate(expr: &Expr) -> Result<&str> {
    match expr {
        Expr::Quote(inner) => inner
            .as_symbol()
            .ok_or_else(|| anyhow!("expected predicate as a quoted symbol")),
        Expr::Symbol(value) => Ok(value),
        _ => bail!("expected predicate as a quoted symbol"),
    }
}

fn escape_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// Body-position operators that aren't relation references. Logical
/// compounds descend into their children; comparison and `between` are
/// terminal (their args are values, not predicate names).
pub fn is_logical_body_op(name: &str) -> bool {
    matches!(name, "and" | "or" | "not")
}

pub fn is_cmp_body_op(name: &str) -> bool {
    matches!(name, "<" | "<=" | ">" | ">=" | "=" | "!=" | "between")
}

/// Aggregate forms: `(sum ?v pred [col] [by ?k key_col ...])`. The
/// second argument is the source relation and gets validated; everything
/// else is a var or column index.
pub fn is_aggregate_body_op(name: &str) -> bool {
    matches!(name, "count" | "sum" | "min" | "max" | "avg")
}

/// Which `(find ...)` columns of an expanded query hold interned ids.
///
/// Rayforce strips the datom tag from every I64 result column, so a cell no
/// longer says whether it is an interned string/symbol id or a plain number —
/// only the query does. A variable is an id when a positive body atom binds
/// it in an id slot: any slot of a triple pattern, any slot of an EDB except
/// the `facts_i64` value, or a rule argument that is itself an id. The rest
/// (the `facts_i64` value, `count`/`sum`/`avg`, find expressions) are numbers.
// ponytail: a variable bound only through `(= ?x ?y)` reads as a number even
// when ?y is an id; follow equalities if a query ever needs that.
pub fn find_columns_are_ids(expanded_query: &str) -> Result<Vec<bool>> {
    let expr = parse_one(expanded_query)?;
    let clause = |name: &str| -> &[Expr] {
        expr.as_list()
            .into_iter()
            .flatten()
            .filter_map(Expr::as_list)
            .find(|c| c.first().and_then(Expr::as_symbol) == Some(name))
            .map_or(&[], |c| &c[1..])
    };

    // Each rule is `((head arg...) body...)`. Head columns only ever flip from
    // number to id, so iterating to a fixpoint settles recursive rules.
    let rules: Vec<(&str, &[Expr], &[Expr])> = clause("rules")
        .iter()
        .filter_map(|rule| {
            let (head, body) = rule.as_list()?.split_first()?;
            let (name, args) = head.as_list()?.split_first()?;
            Some((name.as_symbol()?, args, body))
        })
        .collect();
    let mut rule_cols: HashMap<&str, Vec<bool>> = HashMap::new();
    loop {
        let mut changed = false;
        for (name, args, body) in &rules {
            let ids = id_vars(body, &rule_cols);
            let cols = rule_cols.entry(name).or_default();
            if cols.len() < args.len() {
                cols.resize(args.len(), false);
            }
            for (i, arg) in args.iter().enumerate() {
                if !cols[i] && term_is_id(arg, &ids) {
                    cols[i] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    let ids = id_vars(clause("where"), &rule_cols);
    Ok(clause("find")
        .iter()
        .map(|term| term_is_id(term, &ids))
        .collect())
}

fn term_is_id(term: &Expr, id_vars: &HashSet<&str>) -> bool {
    match term {
        Expr::Symbol(s) if s.starts_with('?') => id_vars.contains(s.as_str()),
        Expr::Symbol(s) => s.parse::<f64>().is_err(),
        Expr::String(_) | Expr::Quote(_) => true,
        Expr::List(_) => false,
    }
}

fn id_vars<'a>(body: &'a [Expr], rule_cols: &HashMap<&str, Vec<bool>>) -> HashSet<&'a str> {
    let mut out = HashSet::new();
    for atom in body {
        collect_id_vars(atom, rule_cols, &mut out);
    }
    out
}

fn collect_id_vars<'a>(
    atom: &'a Expr,
    rule_cols: &HashMap<&str, Vec<bool>>,
    out: &mut HashSet<&'a str>,
) {
    let Some((head, args)) = atom.as_list().and_then(|items| items.split_first()) else {
        return;
    };
    let var = |e: &'a Expr| e.as_symbol().filter(|s| s.starts_with('?'));
    let col_is_id = |rel: &str, col: usize| match rule_cols.get(rel) {
        Some(cols) => cols.get(col).copied().unwrap_or(false),
        None => !(rel == "facts_i64" && col == 2),
    };
    match head.as_symbol() {
        // A negated atom binds nothing.
        Some("not") => {}
        Some(op) if is_logical_body_op(op) => {
            for child in args {
                collect_id_vars(child, rule_cols, out);
            }
        }
        Some(op) if is_cmp_body_op(op) => {}
        // `(agg ?out rel [col] [by ?key key_col ...])`
        Some(op) if is_aggregate_body_op(op) => {
            let Some(rel) = args.get(1).and_then(Expr::as_symbol) else {
                return;
            };
            let id_col = |e: &Expr| {
                e.as_symbol()
                    .and_then(|s| s.parse().ok())
                    .is_some_and(|col| col_is_id(rel, col))
            };
            let by = args.iter().position(|a| a.as_symbol() == Some("by"));
            if matches!(op, "min" | "max") && by != Some(2) && args.get(2).is_some_and(id_col) {
                out.extend(var(&args[0]));
            }
            for key in args[by.map_or(args.len(), |i| i + 1)..].chunks_exact(2) {
                if id_col(&key[1]) {
                    out.extend(var(&key[0]));
                }
            }
        }
        // `(rel ?a ?b ...)`: a rule or EDB call.
        Some(rel) if !rel.starts_with('?') && rel != "_" && rel.parse::<f64>().is_err() => {
            for (col, arg) in args.iter().enumerate() {
                if col_is_id(rel, col) {
                    out.extend(var(arg));
                }
            }
        }
        // `(?e 'attr ?v)`: every slot of a triple pattern is an interned id.
        _ => out.extend(std::iter::once(head).chain(args).filter_map(var)),
    }
}

struct Parser<'a> {
    source: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, index: 0 }
    }

    fn is_eof(&self) -> bool {
        self.index >= self.source.len()
    }

    fn skip_ws_and_comments(&mut self) {
        while self.index < self.source.len() {
            let rest = &self.source[self.index..];
            if rest.starts_with(";;") {
                while self.index < self.source.len() && self.source.as_bytes()[self.index] != b'\n'
                {
                    self.index += 1;
                }
                continue;
            }
            let mut chars = rest.chars();
            let Some(ch) = chars.next() else {
                break;
            };
            if ch.is_whitespace() {
                self.index += ch.len_utf8();
                continue;
            }
            break;
        }
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        self.skip_ws_and_comments();
        let ch = self
            .peek_char()
            .ok_or_else(|| anyhow!("unexpected end of input"))?;
        match ch {
            '(' | '[' => self.parse_list(),
            ')' | ']' => bail!("unexpected closing delimiter"),
            '"' => self.parse_string(),
            '\'' => self.parse_quote(),
            _ => self.parse_symbol(),
        }
    }

    fn parse_list(&mut self) -> Result<Expr> {
        let opener = self
            .next_char()
            .ok_or_else(|| anyhow!("unexpected end of input"))?;
        let closer = match opener {
            '(' => ')',
            '[' => ']',
            _ => bail!("expected list opener"),
        };
        let mut items = Vec::new();
        loop {
            self.skip_ws_and_comments();
            let Some(ch) = self.peek_char() else {
                bail!("unterminated list");
            };
            if ch == closer {
                self.next_char();
                break;
            }
            items.push(self.parse_expr()?);
        }
        Ok(Expr::List(items))
    }

    fn parse_string(&mut self) -> Result<Expr> {
        let quote = self.next_char();
        if quote != Some('"') {
            bail!("expected string literal");
        }
        let mut out = String::new();
        loop {
            let ch = self
                .next_char()
                .ok_or_else(|| anyhow!("unterminated string"))?;
            match ch {
                '"' => break,
                '\\' => {
                    let escaped = self
                        .next_char()
                        .ok_or_else(|| anyhow!("unterminated string escape"))?;
                    out.push(match escaped {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    });
                }
                other => out.push(other),
            }
        }
        Ok(Expr::String(out))
    }

    fn parse_quote(&mut self) -> Result<Expr> {
        self.next_char();
        let inner = self.parse_expr()?;
        Ok(Expr::Quote(Box::new(inner)))
    }

    fn parse_symbol(&mut self) -> Result<Expr> {
        let start = self.index;
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() || matches!(ch, '(' | ')' | '[' | ']' | '"' | '\'' | ';') {
                break;
            }
            self.next_char();
        }
        if self.index == start {
            bail!("expected symbol");
        }
        Ok(Expr::Symbol(self.source[start..self.index].to_string()))
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.index..].chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.index += ch.len_utf8();
        Some(ch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retract_fact_by_id_lowers_without_a_tuple() {
        let lower = |src: &str| {
            let options = LoweringOptions {
                default_query_exom: None,
                default_rule_exom: None,
            };
            lower_top_level(&parse_one(src).unwrap(), options).unwrap()
        };
        let by_id = CanonicalForm::RetractFact(CanonicalFactRetract {
            exom: "main".into(),
            fact_id: "f".into(),
            tuple: None,
        });
        assert_eq!(lower("(retract-fact main \"f\")"), [by_id.clone()]);
        assert_eq!(lower("(in-exom main (retract-fact \"f\"))"), [by_id.clone()]);
        assert_eq!(by_id.emit(), "(retract-fact main \"f\")");
        // `::` paths name the same exom as slash paths.
        assert_eq!(
            lower("(retract-fact work::main \"f\")")[0].emit(),
            "(retract-fact work/main \"f\")"
        );
    }

    #[test]
    fn find_columns_are_ids_follows_rules_and_typed_edbs() {
        let kinds = |q: &str| find_columns_are_ids(q).unwrap();
        // Triple pattern, with either quote spelling for the attribute.
        assert_eq!(
            kinds("(query m (find ?e ?a ?v) (where (?e ?a ?v)))"),
            [true, true, true]
        );
        assert_eq!(
            kinds("(query m (find ?e ?v) (where (?e (quote fact/value) ?v)))"),
            [true, true]
        );
        // Through a rule, including a constant head and a recursive rule.
        assert_eq!(
            kinds(
                "(query m (find ?id ?band ?w ?n) (where (band ?id ?band ?w) (count ?n band)) \
                 (rules ((band ?id \"small\" ?w) (facts_i64 ?id 'weight ?w) (< ?w 60)) \
                        ((up ?a ?b) (?a 'parent ?b)) \
                        ((up ?a ?c) (up ?a ?b) (?b 'parent ?c))))"
            ),
            [true, true, false, false]
        );
        assert_eq!(
            kinds(
                "(query m (find ?a ?c) (where (up ?a ?c)) \
                 (rules ((up ?a ?c) (up ?a ?b) (?b 'parent ?c)) ((up ?a ?b) (?a 'parent ?b))))"
            ),
            [true, true]
        );
        // Aggregates: the result is a number, a group key takes its column's kind.
        assert_eq!(
            kinds("(query m (find ?attr ?avg ?v) (where (avg ?avg facts_i64 2 by ?attr 1 ?v 2)))"),
            [true, false, false]
        );
        // A negated atom binds nothing.
        assert_eq!(
            kinds("(query m (find ?x) (where (not (?x 'a ?y))))"),
            [false]
        );
    }

    fn lowering_options<'a>() -> LoweringOptions<'a> {
        LoweringOptions {
            default_query_exom: None,
            default_rule_exom: Some("main"),
        }
    }

    #[test]
    fn lowers_in_exom_query_to_explicit_query() {
        let expr =
            parse_one("(in-exom main (query (find ?x) (where (fact-row ?x ?p ?v))))").unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        assert_eq!(lowered.len(), 1);
        assert_eq!(
            lowered[0].emit(),
            "(query main (find ?x) (where (fact-row ?x ?p ?v)))"
        );
    }

    #[test]
    fn lowers_multiple_in_exom_forms_in_order() {
        let expr = parse_one(
            "(in-exom main (assert-fact \"f\" 'pred \"v\") (retract-fact \"f\" 'pred \"v\"))",
        )
        .unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        let emitted = lowered.iter().map(CanonicalForm::emit).collect::<Vec<_>>();
        assert_eq!(
            emitted,
            vec![
                "(assert-fact main \"f\" 'pred \"v\")".to_string(),
                "(retract-fact main \"f\" 'pred \"v\")".to_string(),
            ]
        );
    }

    #[test]
    fn conflicting_explicit_exom_inside_in_exom_fails() {
        let expr = parse_one("(in-exom main (query other (find ?x) (where (p ?x))))").unwrap();
        let err = lower_top_level(&expr, lowering_options()).unwrap_err();
        assert!(err
            .to_string()
            .contains("conflicting explicit exom 'other' inside in-exom 'main'"));
    }

    #[test]
    fn unsupported_nested_form_inside_in_exom_fails() {
        let expr = parse_one("(in-exom main (+ 1 2))").unwrap();
        let err = lower_top_level(&expr, lowering_options()).unwrap_err();
        assert!(err
            .to_string()
            .contains("unsupported nested form inside in-exom: +"));
    }

    #[test]
    fn rule_without_explicit_exom_uses_default() {
        let expr = parse_one("(rule (fact-row ?f ?p ?v) (?f 'fact/predicate ?p))").unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        assert_eq!(
            lowered[0].emit(),
            "(rule main (fact-row ?f ?p ?v) (?f 'fact/predicate ?p))"
        );
    }

    #[test]
    fn append_rules_uses_canonical_query_emission() {
        let expr = parse_one("(query main (find ?x) (where (p ?x)))").unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        let CanonicalForm::Query(query) = &lowered[0] else {
            panic!("expected query");
        };
        let expanded = append_rules_to_query(
            query,
            &[String::from(
                "((fact-row ?f ?p ?v) (?f 'fact/predicate ?p) (?f 'fact/value ?v))",
            )],
        )
        .unwrap();
        assert_eq!(
            expanded,
            "(query main (find ?x) (where (p ?x)) (rules ((fact-row ?f ?p ?v) (?f 'fact/predicate ?p) (?f 'fact/value ?v))))"
        );
    }

    #[test]
    fn rewrite_pins_sym_encoded_attribute_literal() {
        let expr =
            parse_one("(query main (find ?f) (where (?f 'fact/predicate \"fx/marker\")))").unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        let CanonicalForm::Query(mut query) = lowered.into_iter().next().unwrap() else {
            panic!("expected query");
        };
        query.rewrite_body_literals_with_schema(|attr| match attr {
            "fact/predicate" => Some("predicate".to_string()),
            _ => None,
        });
        assert_eq!(
            query.emit(),
            "(query main (find ?f) (where (?f 'fact/predicate 'fx/marker)))"
        );
    }

    #[test]
    fn rewrite_skips_string_kind_attribute() {
        let expr = parse_one("(query main (find ?f) (where (?f 'fact/value \"hello\")))").unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        let CanonicalForm::Query(mut query) = lowered.into_iter().next().unwrap() else {
            panic!("expected query");
        };
        query.rewrite_body_literals_with_schema(|attr| match attr {
            "fact/value" => Some("string".to_string()),
            _ => None,
        });
        assert_eq!(
            query.emit(),
            "(query main (find ?f) (where (?f 'fact/value \"hello\")))"
        );
    }

    #[test]
    fn rewrite_descends_into_logical_compounds() {
        let expr = parse_one(
            "(query main (find ?f) (where (and (?f 'fact/predicate \"fx/marker\") (?f 'fact/value \"v\"))))",
        )
        .unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        let CanonicalForm::Query(mut query) = lowered.into_iter().next().unwrap() else {
            panic!("expected query");
        };
        query.rewrite_body_literals_with_schema(|attr| match attr {
            "fact/predicate" => Some("predicate".to_string()),
            "fact/value" => Some("string".to_string()),
            _ => None,
        });
        assert_eq!(
            query.emit(),
            "(query main (find ?f) (where (and (?f 'fact/predicate 'fx/marker) (?f 'fact/value \"v\"))))"
        );
    }

    #[test]
    fn rewrite_skips_unknown_attribute() {
        let expr = parse_one("(query main (find ?f) (where (?f 'user/state \"active\")))").unwrap();
        let lowered = lower_top_level(&expr, lowering_options()).unwrap();
        let CanonicalForm::Query(mut query) = lowered.into_iter().next().unwrap() else {
            panic!("expected query");
        };
        query.rewrite_body_literals_with_schema(|_| None);
        assert_eq!(
            query.emit(),
            "(query main (find ?f) (where (?f 'user/state \"active\")))"
        );
    }
}
