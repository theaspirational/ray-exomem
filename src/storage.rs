//! Safe Rust wrappers over rayforce2 FFI for columnar table persistence.
//!
//! Provides typed table builders and loaders for each Brain entity type,
//! plus RAII wrappers and symbol table helpers.

use std::ffi::CString;
use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

use crate::brain::{Belief, BeliefStatus, Brain, Branch, Fact, Observation, Tx, TxAction};
use crate::datom;
use crate::ffi;
use crate::rayfall_parser::{self, DatomRole};
use crate::system_schema;

// ---------------------------------------------------------------------------
// RAII wrapper for ray_t*
// ---------------------------------------------------------------------------

pub struct RayObj {
    ptr: *mut ffi::ray_t,
}

// Safety: RayObj is always accessed behind a Mutex; raw pointer transfer
// between threads is safe because rayforce2 operations are not re-entrant.
unsafe impl Send for RayObj {}

impl RayObj {
    /// Take ownership of a raw `ray_t*`. The pointer must be non-null.
    pub fn from_raw(ptr: *mut ffi::ray_t) -> Result<Self> {
        if ptr.is_null() {
            bail!("received null ray_t pointer");
        }
        Ok(Self { ptr })
    }

    pub fn as_ptr(&self) -> *mut ffi::ray_t {
        self.ptr
    }

    pub fn try_clone(&self) -> Result<Self> {
        if self.ptr.is_null() {
            bail!("cannot clone null ray_t pointer");
        }
        unsafe { ffi::ray_retain(self.ptr) };
        Ok(Self { ptr: self.ptr })
    }

    /// Release ownership, returning the raw pointer without calling release.
    pub fn into_raw(self) -> *mut ffi::ray_t {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }
}

impl Drop for RayObj {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { ffi::ray_release(self.ptr) };
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol table helpers
// ---------------------------------------------------------------------------

pub fn sym_intern(s: &str) -> i64 {
    unsafe { ffi::ray_sym_intern(s.as_ptr() as *const _, s.len()) }
}

/// Current number of entries in the global rayforce2 symbol table (best-effort diagnostic).
pub fn sym_count() -> u32 {
    unsafe { ffi::ray_sym_count() }
}

pub fn sym_lookup(id: i64) -> Result<String> {
    unsafe {
        let atom = ffi::ray_sym_str(id);
        if atom.is_null() {
            bail!("invalid symbol id {}", id);
        }
        let ptr = ffi::ray_str_ptr(atom);
        let len = ffi::ray_str_len(atom);
        if ptr.is_null() || len == 0 {
            return Ok(String::new());
        }
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

pub fn sym_save(path: &Path) -> Result<()> {
    let c_path = path_to_cstring(path)?;
    let err = unsafe { ffi::ray_sym_save(c_path.as_ptr()) };
    if err != ffi::RAY_OK {
        bail!("ray_sym_save failed (error code {})", err);
    }
    Ok(())
}

/// Load the global symbol table. Returns `Ok(true)` on success or empty,
/// `Ok(false)` if the file is corrupt/incompatible (caller should wipe and
/// start fresh), or `Err` for genuine I/O failures.
pub fn sym_load(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    let c_path = path_to_cstring(path)?;
    let err = unsafe { ffi::ray_sym_load(c_path.as_ptr()) };
    if err == ffi::RAY_OK {
        return Ok(true);
    }
    if err == ffi::RAY_ERR_CORRUPT {
        return Ok(false);
    }
    bail!("ray_sym_load failed (error code {})", err)
}

// ---------------------------------------------------------------------------
// JSONL sidecar persistence
// ---------------------------------------------------------------------------

/// Write items as one-JSON-object-per-line. Uses atomic rename so readers
/// never see a partial file.
pub fn save_jsonl<T: Serialize>(items: &[T], path: &Path) -> Result<()> {
    let tmp = path.with_extension("jsonl.tmp");
    let mut f = std::fs::File::create(&tmp)
        .with_context(|| format!("failed to create {}", tmp.display()))?;
    for item in items {
        serde_json::to_writer(&mut f, item)
            .with_context(|| format!("failed to serialize to {}", tmp.display()))?;
        f.write_all(b"\n")?;
    }
    f.flush()?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Load items from a JSONL file. Returns an empty vec if the file doesn't exist.
pub fn load_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let f =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(f);
    let mut items = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line =
            line.with_context(|| format!("failed to read line {} of {}", i + 1, path.display()))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let item: T = serde_json::from_str(line)
            .with_context(|| format!("failed to parse line {} of {}", i + 1, path.display()))?;
        items.push(item);
    }
    Ok(items)
}

// ---------------------------------------------------------------------------
// Splayed table I/O
// ---------------------------------------------------------------------------

pub fn save_table(table: &RayObj, dir: &Path, sym_path: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create splay dir {}", dir.display()))?;
    let c_dir = path_to_cstring(dir)?;
    let c_sym = path_to_cstring(sym_path)?;
    let err = unsafe { ffi::ray_splay_save(table.as_ptr(), c_dir.as_ptr(), c_sym.as_ptr()) };
    if err != ffi::RAY_OK {
        bail!(
            "ray_splay_save failed for {} (error code {})",
            dir.display(),
            err
        );
    }
    Ok(())
}

pub fn load_table(dir: &Path, sym_path: &Path) -> Result<RayObj> {
    let c_dir = path_to_cstring(dir)?;
    let c_sym = path_to_cstring(sym_path)?;
    let ptr = unsafe { ffi::ray_splay_load(c_dir.as_ptr(), c_sym.as_ptr()) };
    RayObj::from_raw(ptr).with_context(|| format!("ray_splay_load failed for {}", dir.display()))
}

pub fn table_exists(dir: &Path) -> bool {
    dir.join(".d").exists()
}

pub fn encode_string_datom(value: &str) -> i64 {
    let sym_id = sym_intern(value);
    datom::encode_str(sym_id)
}

pub fn encode_symbol_datom(value: &str) -> i64 {
    let sym_id = sym_intern(value);
    datom::encode_sym(sym_id)
}

pub fn decode_datom_to_string(encoded: i64) -> Result<String> {
    let kind = datom::kind(encoded);
    if kind == datom::KIND_I64 {
        return Ok(encoded.to_string());
    }
    let payload = datom::payload(encoded);
    sym_lookup(payload)
}

pub fn decode_query_table(table: &RayObj, query_source: &str) -> Result<Value> {
    let tbl = table.as_ptr();
    let ncols = unsafe { ffi::ray_table_ncols(tbl) };
    let nrows = unsafe { ffi::ray_table_nrows(tbl) };
    let roles = rayfall_parser::datom_query_projection_roles(query_source).unwrap_or_default();

    let mut columns = Vec::with_capacity(ncols as usize);
    let mut raw_types = Vec::with_capacity(ncols as usize);
    let mut rows = Vec::with_capacity(nrows as usize);

    for c in 0..ncols {
        let name_id = unsafe { ffi::ray_table_col_name(tbl, c) };
        columns.push(sym_lookup(name_id).unwrap_or_else(|_| format!("col{c}")));
        let col = unsafe { ffi::ray_table_get_col_idx(tbl, c) };
        if col.is_null() {
            raw_types.push("unknown".to_string());
            continue;
        }
        let typ = unsafe { ffi::ray_obj_type(col) };
        raw_types.push(type_name(typ).to_string());
    }

    for r in 0..nrows {
        let mut row = Vec::with_capacity(ncols as usize);
        for c in 0..ncols {
            let col = unsafe { ffi::ray_table_get_col_idx(tbl, c) };
            if col.is_null() {
                row.push(Value::Null);
                continue;
            }
            if unsafe { ffi::ray_vec_is_null(col, r) } {
                row.push(Value::Null);
                continue;
            }
            let role = roles.get(c as usize).copied().flatten();
            let typ = unsafe { ffi::ray_obj_type(col) };
            row.push(decode_query_cell(col, typ, r, role)?);
        }
        rows.push(Value::Array(row));
    }

    let types = infer_semantic_types(&rows, ncols as usize);

    Ok(json!({
        "columns": columns,
        "raw_types": raw_types,
        "types": types,
        "rows": rows
    }))
}

pub fn format_decoded_query_table(decoded: &Value) -> String {
    let columns = decoded
        .get("columns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let types = decoded
        .get("types")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rows = decoded
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let column_strings: Vec<String> = columns
        .iter()
        .map(|v| v.as_str().unwrap_or("?").to_string())
        .collect();
    let type_strings: Vec<String> = types
        .iter()
        .map(|v| v.as_str().unwrap_or("unknown").to_string())
        .collect();

    let display_rows: Vec<Vec<String>> = rows
        .iter()
        .filter_map(Value::as_array)
        .map(|row| row.iter().map(display_json_value).collect())
        .collect();

    format_string_table(&column_strings, &type_strings, &display_rows)
}

fn type_name(typ: i8) -> &'static str {
    match typ {
        ffi::RAY_BOOL => "bool",
        ffi::RAY_U8 => "u8",
        ffi::RAY_I16 => "i16",
        ffi::RAY_I32 => "i32",
        ffi::RAY_I64 => "i64",
        ffi::RAY_F32 => "f32",
        ffi::RAY_F64 => "f64",
        ffi::RAY_DATE => "date",
        ffi::RAY_TIME => "time",
        ffi::RAY_TIMESTAMP => "timestamp",
        ffi::RAY_SYM => "sym",
        ffi::RAY_STR => "str",
        ffi::RAY_TABLE => "table",
        _ => "unknown",
    }
}

fn infer_semantic_types(rows: &[Value], ncols: usize) -> Vec<String> {
    let mut types = vec!["null".to_string(); ncols];
    for col_idx in 0..ncols {
        let mut inferred = "null";
        for row in rows {
            let Some(arr) = row.as_array() else {
                continue;
            };
            let Some(value) = arr.get(col_idx) else {
                continue;
            };
            if value.is_null() {
                continue;
            }
            inferred = match value {
                Value::String(_) => "string",
                Value::Bool(_) => "bool",
                Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
                Value::Number(_) => "number",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
                _ => "unknown",
            };
            break;
        }
        types[col_idx] = inferred.to_string();
    }
    types
}

fn display_json_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => value.to_string(),
    }
}

fn format_string_table(columns: &[String], types: &[String], rows: &[Vec<String>]) -> String {
    if columns.is_empty() {
        return "(empty)".to_string();
    }

    let ncols = columns.len();
    let mut widths = vec![0usize; ncols];
    for i in 0..ncols {
        widths[i] = columns[i]
            .len()
            .max(types.get(i).map(|s| s.len()).unwrap_or(0));
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(ncols) {
            widths[i] = widths[i].max(cell.len());
        }
    }

    let mut out = String::new();
    let border = |left: &str, fill: &str, sep: &str, right: &str, widths: &[usize]| -> String {
        let mut line = String::from(left);
        for (i, width) in widths.iter().enumerate() {
            line.push_str(&fill.repeat(*width + 2));
            line.push_str(if i + 1 == widths.len() { right } else { sep });
        }
        line
    };

    out.push_str(&border("┌", "─", "┬", "┐", &widths));
    out.push('\n');
    out.push_str(&format_table_row(columns, &widths));
    out.push('\n');
    out.push_str(&format_table_row(types, &widths));
    out.push('\n');
    out.push_str(&border("├", "─", "┼", "┤", &widths));
    out.push('\n');
    for row in rows {
        out.push_str(&format_table_row(row, &widths));
        out.push('\n');
    }
    out.push_str(&border("└", "─", "┴", "┘", &widths));
    out
}

fn format_table_row(cells: &[String], widths: &[usize]) -> String {
    let mut line = String::from("│");
    for (i, width) in widths.iter().enumerate() {
        let cell = cells.get(i).map(String::as_str).unwrap_or("");
        line.push(' ');
        line.push_str(cell);
        if *width > cell.len() {
            line.push_str(&" ".repeat(*width - cell.len()));
        }
        line.push(' ');
        line.push('│');
    }
    line
}

fn decode_query_cell(
    col: *mut ffi::ray_t,
    typ: i8,
    row: i64,
    role: Option<DatomRole>,
) -> Result<Value> {
    match typ {
        ffi::RAY_STR => {
            let mut len = 0usize;
            let ptr = unsafe { ffi::ray_str_vec_get(col, row, &mut len as *mut usize) };
            if ptr.is_null() {
                Ok(Value::Null)
            } else {
                let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
                Ok(json!(String::from_utf8_lossy(bytes).into_owned()))
            }
        }
        ffi::RAY_SYM => {
            let sym_id = unsafe { ffi::ray_vec_get_sym_id(col, row) };
            Ok(json!(
                sym_lookup(sym_id).unwrap_or_else(|_| sym_id.to_string())
            ))
        }
        ffi::RAY_F32 | ffi::RAY_F64 => {
            let value = unsafe { ffi::ray_vec_get_f64(col, row) };
            Ok(json!(value))
        }
        ffi::RAY_BOOL
        | ffi::RAY_U8
        | ffi::RAY_I16
        | ffi::RAY_I32
        | ffi::RAY_I64
        | ffi::RAY_DATE
        | ffi::RAY_TIME
        | ffi::RAY_TIMESTAMP => {
            let value = unsafe { ffi::ray_vec_get_i64(col, row) };
            decode_i64_query_cell(value, role)
        }
        _ => Ok(json!(format!("<{}>", type_name(typ)))),
    }
}

fn decode_i64_query_cell(value: i64, role: Option<DatomRole>) -> Result<Value> {
    match role {
        Some(DatomRole::Entity) | Some(DatomRole::Value) => {
            Ok(json!(decode_datom_to_string(value)?))
        }
        Some(DatomRole::Attribute) => Ok(json!(sym_lookup(value)?)),
        None => {
            if datom::kind(value) != datom::KIND_I64 {
                Ok(json!(decode_datom_to_string(value)?))
            } else {
                Ok(json!(value))
            }
        }
    }
}

fn tx_entity_id(tx_id: u64) -> String {
    format!("tx/{}", tx_id)
}

fn branch_entity_id(branch_id: &str) -> String {
    format!("branch/{}", branch_id)
}

fn string_num<T: std::fmt::Display>(value: T) -> String {
    value.to_string()
}

fn push_datom_row(
    e_col: &mut *mut ffi::ray_t,
    a_col: &mut *mut ffi::ray_t,
    v_col: &mut *mut ffi::ray_t,
    entity: &str,
    attribute: &str,
    value: &str,
) -> Result<()> {
    let entity = encode_string_datom(entity);
    let attribute = sym_intern(attribute);
    let value = encode_string_datom(value);

    unsafe {
        *e_col = ffi::ray_vec_append(*e_col, &entity as *const i64 as *const _);
        *a_col = ffi::ray_vec_append(*a_col, &attribute as *const i64 as *const _);
        *v_col = ffi::ray_vec_append(*v_col, &value as *const i64 as *const _);
    }

    if (*e_col).is_null() || (*a_col).is_null() || (*v_col).is_null() {
        bail!("failed to append datom row");
    }

    Ok(())
}

fn push_datom_row_with_encoded_value(
    e_col: &mut *mut ffi::ray_t,
    a_col: &mut *mut ffi::ray_t,
    v_col: &mut *mut ffi::ray_t,
    entity: &str,
    attribute: &str,
    encoded_value: i64,
) -> Result<()> {
    let entity = encode_string_datom(entity);
    let attribute = sym_intern(attribute);

    unsafe {
        *e_col = ffi::ray_vec_append(*e_col, &entity as *const i64 as *const _);
        *a_col = ffi::ray_vec_append(*a_col, &attribute as *const i64 as *const _);
        *v_col = ffi::ray_vec_append(*v_col, &encoded_value as *const i64 as *const _);
    }

    if (*e_col).is_null() || (*a_col).is_null() || (*v_col).is_null() {
        bail!("failed to append datom row");
    }

    Ok(())
}

pub fn build_datoms_table(brain: &Brain) -> Result<RayObj> {
    let facts = brain.current_facts();
    let txs = brain.current_transactions();
    let observations = brain.observations();
    let beliefs = brain.current_beliefs();
    let branches = brain.branches();
    let mut row_count = facts.len();
    for fact in &facts {
        row_count += 6; // fact/predicate, fact/value, confidence, provenance, valid_from, created_by
        if fact.valid_to.is_some() {
            row_count += 1;
        }
        if fact.superseded_by_tx.is_some() {
            row_count += 1;
        }
        if fact.revoked_by_tx.is_some() {
            row_count += 1;
        }
    }
    for tx in &txs {
        row_count += 4; // tx/id, tx/time, tx/actor, tx/action
        row_count += 1; // tx/branch
        if tx.parent_tx_id.is_some() {
            row_count += 1;
        }
        if tx.session.is_some() {
            row_count += 1;
        }
        row_count += tx.refs.len();
        if matches!(tx.action, TxAction::Merge) && tx.refs.len() >= 2 {
            row_count += 2;
        }
    }
    for obs in observations {
        row_count += 7; // source_type, source_ref, content, created_at, confidence, tx, valid_from
        if obs.valid_to.is_some() {
            row_count += 1;
        }
        row_count += obs.tags.len();
    }
    for belief in &beliefs {
        row_count += 6; // claim, status, confidence, created_by, valid_from, rationale
        if belief.valid_to.is_some() {
            row_count += 1;
        }
        row_count += belief.supported_by.len();
    }
    for branch in branches {
        row_count += 4; // id, name, archived, created_by
        if branch.parent_branch_id.is_some() {
            row_count += 1;
        }
    }

    unsafe {
        let tbl = ffi::ray_table_new(3);
        if tbl.is_null() {
            bail!("failed to allocate datoms table");
        }

        let mut e_col = ffi::ray_vec_new(ffi::RAY_I64, row_count as i64);
        let mut a_col = ffi::ray_vec_new(ffi::RAY_SYM, row_count as i64);
        let mut v_col = ffi::ray_vec_new(ffi::RAY_I64, row_count as i64);
        if e_col.is_null() || a_col.is_null() || v_col.is_null() {
            if !e_col.is_null() {
                ffi::ray_release(e_col);
            }
            if !a_col.is_null() {
                ffi::ray_release(a_col);
            }
            if !v_col.is_null() {
                ffi::ray_release(v_col);
            }
            ffi::ray_release(tbl);
            bail!("failed to allocate datom columns");
        }

        for fact in facts {
            if let Err(err) = (|| -> Result<()> {
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &fact.fact_id,
                    &fact.predicate,
                    &fact.value,
                )?;
                push_datom_row_with_encoded_value(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &fact.fact_id,
                    system_schema::attrs::fact::PREDICATE,
                    encode_symbol_datom(&fact.predicate),
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &fact.fact_id,
                    system_schema::attrs::fact::VALUE,
                    &fact.value,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &fact.fact_id,
                    system_schema::attrs::fact::CONFIDENCE,
                    &string_num(fact.confidence),
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &fact.fact_id,
                    system_schema::attrs::fact::PROVENANCE,
                    &fact.provenance,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &fact.fact_id,
                    system_schema::attrs::fact::VALID_FROM,
                    &fact.valid_from,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &fact.fact_id,
                    system_schema::attrs::fact::CREATED_BY,
                    &tx_entity_id(fact.created_by_tx),
                )?;
                if let Some(valid_to) = fact.valid_to.as_deref() {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &fact.fact_id,
                        system_schema::attrs::fact::VALID_TO,
                        valid_to,
                    )?;
                }
                if let Some(tx_id) = fact.superseded_by_tx {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &fact.fact_id,
                        system_schema::attrs::fact::SUPERSEDED_BY,
                        &tx_entity_id(tx_id),
                    )?;
                }
                if let Some(tx_id) = fact.revoked_by_tx {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &fact.fact_id,
                        system_schema::attrs::fact::REVOKED_BY,
                        &tx_entity_id(tx_id),
                    )?;
                }
                Ok(())
            })() {
                ffi::ray_release(e_col);
                ffi::ray_release(a_col);
                ffi::ray_release(v_col);
                ffi::ray_release(tbl);
                return Err(err);
            }
        }

        for tx in txs {
            let tx_entity = tx_entity_id(tx.tx_id);
            let tx_id_value = tx.tx_id.to_string();
            let action = tx.action.to_string();
            if let Err(err) = (|| -> Result<()> {
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &tx_entity,
                    system_schema::attrs::tx::ID,
                    &tx_id_value,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &tx_entity,
                    system_schema::attrs::tx::TIME,
                    &tx.tx_time,
                )?;
                if let Some(ref email) = tx.user_email {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &tx_entity,
                        system_schema::attrs::tx::USER_EMAIL,
                        email,
                    )?;
                }
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &tx_entity,
                    system_schema::attrs::tx::ACTOR,
                    &tx.actor,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &tx_entity,
                    system_schema::attrs::tx::ACTION,
                    &action,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &tx_entity,
                    system_schema::attrs::tx::BRANCH,
                    &tx.branch_id,
                )?;
                if let Some(parent_tx_id) = tx.parent_tx_id {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &tx_entity,
                        system_schema::attrs::tx::PARENT,
                        &tx_entity_id(parent_tx_id),
                    )?;
                }
                if let Some(session) = tx.session.as_deref() {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &tx_entity,
                        system_schema::attrs::tx::SESSION,
                        session,
                    )?;
                }
                for ref_id in &tx.refs {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &tx_entity,
                        system_schema::attrs::tx::REF,
                        ref_id,
                    )?;
                }
                if matches!(tx.action, TxAction::Merge) && tx.refs.len() >= 2 {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &tx_entity,
                        system_schema::attrs::tx::MERGE_SOURCE,
                        &tx.refs[0],
                    )?;
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &tx_entity,
                        system_schema::attrs::tx::MERGE_TARGET,
                        &tx.refs[1],
                    )?;
                }
                Ok(())
            })() {
                ffi::ray_release(e_col);
                ffi::ray_release(a_col);
                ffi::ray_release(v_col);
                ffi::ray_release(tbl);
                return Err(err);
            }
        }

        for obs in observations {
            if let Err(err) = (|| -> Result<()> {
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &obs.obs_id,
                    system_schema::attrs::observation::SOURCE_TYPE,
                    &obs.source_type,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &obs.obs_id,
                    system_schema::attrs::observation::SOURCE_REF,
                    &obs.source_ref,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &obs.obs_id,
                    system_schema::attrs::observation::CONTENT,
                    &obs.content,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &obs.obs_id,
                    system_schema::attrs::observation::CREATED_AT,
                    &obs.created_at,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &obs.obs_id,
                    system_schema::attrs::observation::CONFIDENCE,
                    &string_num(obs.confidence),
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &obs.obs_id,
                    system_schema::attrs::observation::TX,
                    &tx_entity_id(obs.tx_id),
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &obs.obs_id,
                    system_schema::attrs::observation::VALID_FROM,
                    &obs.valid_from,
                )?;
                if let Some(valid_to) = obs.valid_to.as_deref() {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &obs.obs_id,
                        system_schema::attrs::observation::VALID_TO,
                        valid_to,
                    )?;
                }
                for tag in &obs.tags {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &obs.obs_id,
                        system_schema::attrs::observation::TAG,
                        tag,
                    )?;
                }
                Ok(())
            })() {
                ffi::ray_release(e_col);
                ffi::ray_release(a_col);
                ffi::ray_release(v_col);
                ffi::ray_release(tbl);
                return Err(err);
            }
        }

        for belief in beliefs {
            if let Err(err) = (|| -> Result<()> {
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &belief.belief_id,
                    system_schema::attrs::belief::CLAIM_TEXT,
                    &belief.claim_text,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &belief.belief_id,
                    system_schema::attrs::belief::STATUS,
                    &belief.status.to_string(),
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &belief.belief_id,
                    system_schema::attrs::belief::CONFIDENCE,
                    &string_num(belief.confidence),
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &belief.belief_id,
                    system_schema::attrs::belief::CREATED_BY,
                    &tx_entity_id(belief.created_by_tx),
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &belief.belief_id,
                    system_schema::attrs::belief::VALID_FROM,
                    &belief.valid_from,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &belief.belief_id,
                    system_schema::attrs::belief::RATIONALE,
                    &belief.rationale,
                )?;
                if let Some(valid_to) = belief.valid_to.as_deref() {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &belief.belief_id,
                        system_schema::attrs::belief::VALID_TO,
                        valid_to,
                    )?;
                }
                for support in &belief.supported_by {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &belief.belief_id,
                        system_schema::attrs::belief::SUPPORTS,
                        support,
                    )?;
                }
                Ok(())
            })() {
                ffi::ray_release(e_col);
                ffi::ray_release(a_col);
                ffi::ray_release(v_col);
                ffi::ray_release(tbl);
                return Err(err);
            }
        }

        for branch in branches {
            let branch_entity = branch_entity_id(&branch.branch_id);
            if let Err(err) = (|| -> Result<()> {
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &branch_entity,
                    system_schema::attrs::branch::ID,
                    &branch.branch_id,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &branch_entity,
                    system_schema::attrs::branch::NAME,
                    &branch.name,
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &branch_entity,
                    system_schema::attrs::branch::ARCHIVED,
                    if branch.archived { "true" } else { "false" },
                )?;
                push_datom_row(
                    &mut e_col,
                    &mut a_col,
                    &mut v_col,
                    &branch_entity,
                    system_schema::attrs::branch::CREATED_BY,
                    &tx_entity_id(branch.created_tx_id),
                )?;
                if let Some(parent) = branch.parent_branch_id.as_deref() {
                    push_datom_row(
                        &mut e_col,
                        &mut a_col,
                        &mut v_col,
                        &branch_entity,
                        system_schema::attrs::branch::PARENT,
                        &branch_entity_id(parent),
                    )?;
                }
                Ok(())
            })() {
                ffi::ray_release(e_col);
                ffi::ray_release(a_col);
                ffi::ray_release(v_col);
                ffi::ray_release(tbl);
                return Err(err);
            }
        }

        let tbl = ffi::ray_table_add_col(tbl, sym_intern("e"), e_col);
        ffi::ray_release(e_col);
        let tbl = ffi::ray_table_add_col(tbl, sym_intern("a"), a_col);
        ffi::ray_release(a_col);
        let tbl = ffi::ray_table_add_col(tbl, sym_intern("v"), v_col);
        ffi::ray_release(v_col);
        RayObj::from_raw(tbl)
    }
}

// ---------------------------------------------------------------------------
// Column builder helpers
// ---------------------------------------------------------------------------

struct TableBuilder {
    tbl: *mut ffi::ray_t,
}

impl TableBuilder {
    fn new(ncols: usize) -> Self {
        let tbl = unsafe { ffi::ray_table_new(ncols as i64) };
        Self { tbl }
    }

    fn add_i64_col(&mut self, name: &str, values: &[i64], nulls: Option<&[bool]>) {
        unsafe {
            let col = ffi::ray_vec_new(ffi::RAY_I64, values.len() as i64);
            let mut col = col;
            for (i, &v) in values.iter().enumerate() {
                col = ffi::ray_vec_append(col, &v as *const i64 as *const _);
                if let Some(null_flags) = nulls {
                    if null_flags[i] {
                        ffi::ray_vec_set_null(col, i as i64, true);
                    }
                }
            }
            let name_id = sym_intern(name);
            self.tbl = ffi::ray_table_add_col(self.tbl, name_id, col);
            ffi::ray_release(col);
        }
    }

    fn add_f64_col(&mut self, name: &str, values: &[f64]) {
        unsafe {
            let col = ffi::ray_vec_new(ffi::RAY_F64, values.len() as i64);
            let mut col = col;
            for &v in values {
                col = ffi::ray_vec_append(col, &v as *const f64 as *const _);
            }
            let name_id = sym_intern(name);
            self.tbl = ffi::ray_table_add_col(self.tbl, name_id, col);
            ffi::ray_release(col);
        }
    }

    fn add_sym_col(&mut self, name: &str, values: &[&str]) {
        unsafe {
            let col = ffi::ray_vec_new(ffi::RAY_SYM, values.len() as i64);
            let mut col = col;
            for &s in values {
                let id = sym_intern(s);
                col = ffi::ray_vec_append(col, &id as *const i64 as *const _);
            }
            let name_id = sym_intern(name);
            self.tbl = ffi::ray_table_add_col(self.tbl, name_id, col);
            ffi::ray_release(col);
        }
    }

    fn add_sym_col_nullable(&mut self, name: &str, values: &[Option<&str>]) {
        unsafe {
            let col = ffi::ray_vec_new(ffi::RAY_SYM, values.len() as i64);
            let mut col = col;
            for (i, v) in values.iter().enumerate() {
                let id = sym_intern(v.unwrap_or(""));
                col = ffi::ray_vec_append(col, &id as *const i64 as *const _);
                if v.is_none() {
                    ffi::ray_vec_set_null(col, i as i64, true);
                }
            }
            let name_id = sym_intern(name);
            self.tbl = ffi::ray_table_add_col(self.tbl, name_id, col);
            ffi::ray_release(col);
        }
    }

    /// Add a string column stored as SYM (all strings interned in the global symbol table).
    /// Use for both short repeated strings and longer text — RAY_STR is not supported
    /// by the splayed table serializer.
    fn add_str_col(&mut self, name: &str, values: &[&str]) {
        // Delegate to add_sym_col — all strings are interned as symbols.
        self.add_sym_col(name, values);
    }

    fn finish(self) -> RayObj {
        // Safety: tbl is always non-null from ray_table_new
        RayObj { ptr: self.tbl }
    }
}

// ---------------------------------------------------------------------------
// Column reader helpers
// ---------------------------------------------------------------------------

fn read_i64_col(tbl: *mut ffi::ray_t, col_idx: i64, nrows: i64) -> Vec<i64> {
    unsafe {
        let col = ffi::ray_table_get_col_idx(tbl, col_idx);
        (0..nrows)
            .map(|i| {
                let p = ffi::ray_vec_get(col, i) as *const i64;
                *p
            })
            .collect()
    }
}

fn read_i64_nullable_col(tbl: *mut ffi::ray_t, col_idx: i64, nrows: i64) -> Vec<Option<i64>> {
    unsafe {
        let col = ffi::ray_table_get_col_idx(tbl, col_idx);
        (0..nrows)
            .map(|i| {
                if ffi::ray_vec_is_null(col, i) {
                    None
                } else {
                    let p = ffi::ray_vec_get(col, i) as *const i64;
                    Some(*p)
                }
            })
            .collect()
    }
}

fn read_f64_col(tbl: *mut ffi::ray_t, col_idx: i64, nrows: i64) -> Vec<f64> {
    unsafe {
        let col = ffi::ray_table_get_col_idx(tbl, col_idx);
        (0..nrows)
            .map(|i| {
                let p = ffi::ray_vec_get(col, i) as *const f64;
                *p
            })
            .collect()
    }
}

fn read_sym_col(tbl: *mut ffi::ray_t, col_idx: i64, nrows: i64) -> Result<Vec<String>> {
    unsafe {
        let col = ffi::ray_table_get_col_idx(tbl, col_idx);
        let mut out = Vec::with_capacity(nrows as usize);
        for i in 0..nrows {
            let p = ffi::ray_vec_get(col, i);
            // SYM stores the intern ID; width depends on attrs but we read as the actual stored width.
            // For W32 (most common), the value is a u32. For safety, read the i64 representation.
            let id = *(p as *const i64);
            out.push(sym_lookup(id)?);
        }
        Ok(out)
    }
}

fn read_sym_nullable_col(
    tbl: *mut ffi::ray_t,
    col_idx: i64,
    nrows: i64,
) -> Result<Vec<Option<String>>> {
    unsafe {
        let col = ffi::ray_table_get_col_idx(tbl, col_idx);
        let mut out = Vec::with_capacity(nrows as usize);
        for i in 0..nrows {
            if ffi::ray_vec_is_null(col, i) {
                out.push(None);
            } else {
                let p = ffi::ray_vec_get(col, i);
                let id = *(p as *const i64);
                out.push(Some(sym_lookup(id)?));
            }
        }
        Ok(out)
    }
}

/// Read a column stored as SYM back to strings (used for all text columns).
fn read_str_col(tbl: *mut ffi::ray_t, col_idx: i64, nrows: i64) -> Vec<String> {
    // All text columns are stored as SYM, so delegate to sym reader.
    // Unwrap is safe here since sym_lookup only fails for invalid IDs
    // which shouldn't exist in a valid table.
    read_sym_col(tbl, col_idx, nrows).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Vec<String> encoding (semicolon-delimited)
// ---------------------------------------------------------------------------

fn encode_string_vec(v: &[String]) -> String {
    v.join(";")
}

fn decode_string_vec(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split(';').map(|p| p.to_string()).collect()
    }
}

// ---------------------------------------------------------------------------
// Fact table
// ---------------------------------------------------------------------------

pub fn build_fact_table(facts: &[Fact]) -> RayObj {
    let mut b = TableBuilder::new(11);

    let fact_ids: Vec<&str> = facts.iter().map(|f| f.fact_id.as_str()).collect();
    let predicates: Vec<&str> = facts.iter().map(|f| f.predicate.as_str()).collect();
    let values: Vec<&str> = facts.iter().map(|f| f.value.as_str()).collect();
    let created_ats: Vec<&str> = facts.iter().map(|f| f.created_at.as_str()).collect();
    let created_by: Vec<i64> = facts.iter().map(|f| f.created_by_tx as i64).collect();
    let superseded: Vec<i64> = facts
        .iter()
        .map(|f| f.superseded_by_tx.unwrap_or(0) as i64)
        .collect();
    let superseded_nulls: Vec<bool> = facts.iter().map(|f| f.superseded_by_tx.is_none()).collect();
    let revoked: Vec<i64> = facts
        .iter()
        .map(|f| f.revoked_by_tx.unwrap_or(0) as i64)
        .collect();
    let revoked_nulls: Vec<bool> = facts.iter().map(|f| f.revoked_by_tx.is_none()).collect();
    let confidences: Vec<f64> = facts.iter().map(|f| f.confidence).collect();
    let provenances: Vec<&str> = facts.iter().map(|f| f.provenance.as_str()).collect();
    let valid_froms: Vec<&str> = facts.iter().map(|f| f.valid_from.as_str()).collect();
    let valid_tos: Vec<Option<&str>> = facts.iter().map(|f| f.valid_to.as_deref()).collect();

    b.add_sym_col("fact_id", &fact_ids);
    b.add_sym_col("predicate", &predicates);
    b.add_str_col("value", &values);
    b.add_str_col("created_at", &created_ats);
    b.add_i64_col("created_by_tx", &created_by, None);
    b.add_i64_col("superseded_by_tx", &superseded, Some(&superseded_nulls));
    b.add_i64_col("revoked_by_tx", &revoked, Some(&revoked_nulls));
    b.add_f64_col("confidence", &confidences);
    b.add_sym_col("provenance", &provenances);
    b.add_str_col("valid_from", &valid_froms);
    b.add_sym_col_nullable("valid_to", &valid_tos);

    b.finish()
}

pub fn load_facts(table: &RayObj) -> Result<Vec<Fact>> {
    let tbl = table.as_ptr();
    let nrows = unsafe { ffi::ray_table_nrows(tbl) };

    let fact_ids = read_sym_col(tbl, 0, nrows)?;
    let predicates = read_sym_col(tbl, 1, nrows)?;
    let values = read_str_col(tbl, 2, nrows);
    let created_ats = read_str_col(tbl, 3, nrows);
    let created_by = read_i64_col(tbl, 4, nrows);
    let superseded = read_i64_nullable_col(tbl, 5, nrows);
    let revoked = read_i64_nullable_col(tbl, 6, nrows);
    let confidences = read_f64_col(tbl, 7, nrows);
    let provenances = read_sym_col(tbl, 8, nrows)?;
    let valid_froms = read_str_col(tbl, 9, nrows);
    let valid_tos = read_sym_nullable_col(tbl, 10, nrows)?;

    let mut facts = Vec::with_capacity(nrows as usize);
    for i in 0..nrows as usize {
        facts.push(Fact {
            fact_id: fact_ids[i].clone(),
            predicate: predicates[i].clone(),
            value: values[i].clone(),
            created_at: created_ats[i].clone(),
            created_by_tx: created_by[i] as u64,
            superseded_by_tx: superseded[i].map(|v| v as u64),
            revoked_by_tx: revoked[i].map(|v| v as u64),
            confidence: confidences[i],
            provenance: provenances[i].clone(),
            valid_from: valid_froms[i].clone(),
            valid_to: valid_tos[i].clone(),
        });
    }
    Ok(facts)
}

// ---------------------------------------------------------------------------
// Observation table
// ---------------------------------------------------------------------------

pub fn build_observation_table(observations: &[Observation]) -> RayObj {
    let mut b = TableBuilder::new(10);

    let obs_ids: Vec<&str> = observations.iter().map(|o| o.obs_id.as_str()).collect();
    let source_types: Vec<&str> = observations
        .iter()
        .map(|o| o.source_type.as_str())
        .collect();
    let source_refs: Vec<&str> = observations.iter().map(|o| o.source_ref.as_str()).collect();
    let contents: Vec<&str> = observations.iter().map(|o| o.content.as_str()).collect();
    let created_ats: Vec<&str> = observations.iter().map(|o| o.created_at.as_str()).collect();
    let confidences: Vec<f64> = observations.iter().map(|o| o.confidence).collect();
    let tx_ids: Vec<i64> = observations.iter().map(|o| o.tx_id as i64).collect();
    let tags: Vec<String> = observations
        .iter()
        .map(|o| encode_string_vec(&o.tags))
        .collect();
    let tags_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let valid_froms: Vec<&str> = observations.iter().map(|o| o.valid_from.as_str()).collect();
    let valid_tos: Vec<Option<&str>> = observations.iter().map(|o| o.valid_to.as_deref()).collect();

    b.add_sym_col("obs_id", &obs_ids);
    b.add_sym_col("source_type", &source_types);
    b.add_str_col("source_ref", &source_refs);
    b.add_str_col("content", &contents);
    b.add_str_col("created_at", &created_ats);
    b.add_f64_col("confidence", &confidences);
    b.add_i64_col("tx_id", &tx_ids, None);
    b.add_str_col("tags", &tags_refs);
    b.add_str_col("valid_from", &valid_froms);
    b.add_sym_col_nullable("valid_to", &valid_tos);

    b.finish()
}

pub fn load_observations(table: &RayObj) -> Result<Vec<Observation>> {
    let tbl = table.as_ptr();
    let nrows = unsafe { ffi::ray_table_nrows(tbl) };

    let obs_ids = read_sym_col(tbl, 0, nrows)?;
    let source_types = read_sym_col(tbl, 1, nrows)?;
    let source_refs = read_str_col(tbl, 2, nrows);
    let contents = read_str_col(tbl, 3, nrows);
    let created_ats = read_str_col(tbl, 4, nrows);
    let confidences = read_f64_col(tbl, 5, nrows);
    let tx_ids = read_i64_col(tbl, 6, nrows);
    let tags_raw = read_str_col(tbl, 7, nrows);
    let valid_froms = read_str_col(tbl, 8, nrows);
    let valid_tos = read_sym_nullable_col(tbl, 9, nrows)?;

    let mut obs = Vec::with_capacity(nrows as usize);
    for i in 0..nrows as usize {
        obs.push(Observation {
            obs_id: obs_ids[i].clone(),
            source_type: source_types[i].clone(),
            source_ref: source_refs[i].clone(),
            content: contents[i].clone(),
            created_at: created_ats[i].clone(),
            confidence: confidences[i],
            tx_id: tx_ids[i] as u64,
            tags: decode_string_vec(&tags_raw[i]),
            valid_from: valid_froms[i].clone(),
            valid_to: valid_tos[i].clone(),
        });
    }
    Ok(obs)
}

// ---------------------------------------------------------------------------
// Belief table
// ---------------------------------------------------------------------------

pub fn build_belief_table(beliefs: &[Belief]) -> RayObj {
    let mut b = TableBuilder::new(9);

    let ids: Vec<&str> = beliefs.iter().map(|b| b.belief_id.as_str()).collect();
    let claims: Vec<&str> = beliefs.iter().map(|b| b.claim_text.as_str()).collect();
    let statuses: Vec<&str> = beliefs
        .iter()
        .map(|b| match b.status {
            BeliefStatus::Active => "active",
            BeliefStatus::Superseded => "superseded",
            BeliefStatus::Revoked => "revoked",
        })
        .collect();
    let confidences: Vec<f64> = beliefs.iter().map(|b| b.confidence).collect();
    let supported: Vec<String> = beliefs
        .iter()
        .map(|b| encode_string_vec(&b.supported_by))
        .collect();
    let supported_refs: Vec<&str> = supported.iter().map(|s| s.as_str()).collect();
    let created_by: Vec<i64> = beliefs.iter().map(|b| b.created_by_tx as i64).collect();
    let valid_froms: Vec<&str> = beliefs.iter().map(|b| b.valid_from.as_str()).collect();
    let valid_tos: Vec<Option<&str>> = beliefs.iter().map(|b| b.valid_to.as_deref()).collect();
    let rationales: Vec<&str> = beliefs.iter().map(|b| b.rationale.as_str()).collect();

    b.add_sym_col("belief_id", &ids);
    b.add_str_col("claim_text", &claims);
    b.add_sym_col("status", &statuses);
    b.add_f64_col("confidence", &confidences);
    b.add_str_col("supported_by", &supported_refs);
    b.add_i64_col("created_by_tx", &created_by, None);
    b.add_str_col("valid_from", &valid_froms);
    b.add_sym_col_nullable("valid_to", &valid_tos);
    b.add_str_col("rationale", &rationales);

    b.finish()
}

pub fn load_beliefs(table: &RayObj) -> Result<Vec<Belief>> {
    let tbl = table.as_ptr();
    let nrows = unsafe { ffi::ray_table_nrows(tbl) };

    let ids = read_sym_col(tbl, 0, nrows)?;
    let claims = read_str_col(tbl, 1, nrows);
    let statuses = read_sym_col(tbl, 2, nrows)?;
    let confidences = read_f64_col(tbl, 3, nrows);
    let supported_raw = read_str_col(tbl, 4, nrows);
    let created_by = read_i64_col(tbl, 5, nrows);
    let valid_froms = read_str_col(tbl, 6, nrows);
    let valid_tos = read_sym_nullable_col(tbl, 7, nrows)?;
    let rationales = read_str_col(tbl, 8, nrows);

    let mut beliefs = Vec::with_capacity(nrows as usize);
    for i in 0..nrows as usize {
        let status = match statuses[i].as_str() {
            "superseded" => BeliefStatus::Superseded,
            "revoked" => BeliefStatus::Revoked,
            _ => BeliefStatus::Active,
        };
        beliefs.push(Belief {
            belief_id: ids[i].clone(),
            claim_text: claims[i].clone(),
            status,
            confidence: confidences[i],
            supported_by: decode_string_vec(&supported_raw[i]),
            created_by_tx: created_by[i] as u64,
            valid_from: valid_froms[i].clone(),
            valid_to: valid_tos[i].clone(),
            rationale: rationales[i].clone(),
        });
    }
    Ok(beliefs)
}

// ---------------------------------------------------------------------------
// Transaction table
// ---------------------------------------------------------------------------

pub fn build_tx_table(txs: &[Tx]) -> RayObj {
    let mut b = TableBuilder::new(10);

    let tx_ids: Vec<i64> = txs.iter().map(|t| t.tx_id as i64).collect();
    let times: Vec<&str> = txs.iter().map(|t| t.tx_time.as_str()).collect();
    let actors: Vec<&str> = txs.iter().map(|t| t.actor.as_str()).collect();
    let actions: Vec<&str> = txs
        .iter()
        .map(|t| match t.action {
            TxAction::AssertObservation => "assert-observation",
            TxAction::AssertFact => "assert-fact",
            TxAction::RetractFact => "retract-fact",
            TxAction::ReviseBelief => "revise-belief",
            TxAction::CreateBranch => "create-branch",
            TxAction::Merge => "merge",
        })
        .collect();
    let refs: Vec<String> = txs.iter().map(|t| encode_string_vec(&t.refs)).collect();
    let refs_strs: Vec<&str> = refs.iter().map(|s| s.as_str()).collect();
    let notes: Vec<&str> = txs.iter().map(|t| t.note.as_str()).collect();
    let parent_ids: Vec<i64> = txs
        .iter()
        .map(|t| t.parent_tx_id.unwrap_or(0) as i64)
        .collect();
    let parent_nulls: Vec<bool> = txs.iter().map(|t| t.parent_tx_id.is_none()).collect();
    let branches: Vec<&str> = txs.iter().map(|t| t.branch_id.as_str()).collect();
    let sessions: Vec<Option<&str>> = txs.iter().map(|t| t.session.as_deref()).collect();
    let user_emails: Vec<Option<&str>> = txs.iter().map(|t| t.user_email.as_deref()).collect();

    b.add_i64_col("tx_id", &tx_ids, None);
    b.add_str_col("tx_time", &times);
    b.add_sym_col("actor", &actors);
    b.add_sym_col("action", &actions);
    b.add_str_col("refs", &refs_strs);
    b.add_str_col("note", &notes);
    b.add_i64_col("parent_tx_id", &parent_ids, Some(&parent_nulls));
    b.add_sym_col("branch_id", &branches);
    b.add_sym_col_nullable("session", &sessions);
    b.add_sym_col_nullable("user_email", &user_emails);

    b.finish()
}

pub fn load_txs(table: &RayObj) -> Result<Vec<Tx>> {
    let tbl = table.as_ptr();
    let nrows = unsafe { ffi::ray_table_nrows(tbl) };
    let ncols = unsafe { ffi::ray_table_ncols(tbl) };

    let tx_ids = read_i64_col(tbl, 0, nrows);
    let times = read_str_col(tbl, 1, nrows);
    let actors = read_sym_col(tbl, 2, nrows)?;
    let actions = read_sym_col(tbl, 3, nrows)?;
    let refs_raw = read_str_col(tbl, 4, nrows);
    let notes = read_str_col(tbl, 5, nrows);
    let parent_ids = read_i64_nullable_col(tbl, 6, nrows);
    let branches = read_sym_col(tbl, 7, nrows)?;
    let sessions = if ncols >= 9 {
        read_sym_nullable_col(tbl, 8, nrows)?
    } else {
        vec![None; nrows as usize]
    };
    let user_emails = if ncols >= 10 {
        read_sym_nullable_col(tbl, 9, nrows)?
    } else {
        vec![None; nrows as usize]
    };

    let mut txs = Vec::with_capacity(nrows as usize);
    for i in 0..nrows as usize {
        let action = match actions[i].as_str() {
            "assert-observation" => TxAction::AssertObservation,
            "assert-fact" => TxAction::AssertFact,
            "retract-fact" => TxAction::RetractFact,
            "revise-belief" => TxAction::ReviseBelief,
            "create-branch" => TxAction::CreateBranch,
            "merge" => TxAction::Merge,
            other => bail!("unknown tx action: {}", other),
        };
        txs.push(Tx {
            tx_id: tx_ids[i] as u64,
            tx_time: times[i].clone(),
            user_email: user_emails[i].clone(),
            actor: actors[i].clone(),
            action,
            refs: decode_string_vec(&refs_raw[i]),
            note: notes[i].clone(),
            parent_tx_id: parent_ids[i].map(|v| v as u64),
            branch_id: branches[i].clone(),
            session: sessions[i].clone(),
        });
    }
    Ok(txs)
}

// ---------------------------------------------------------------------------
// Branch table
// ---------------------------------------------------------------------------

pub fn build_branch_table(branches: &[Branch]) -> RayObj {
    let mut b = TableBuilder::new(5);

    let ids: Vec<&str> = branches.iter().map(|b| b.branch_id.as_str()).collect();
    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    let parents: Vec<Option<&str>> = branches
        .iter()
        .map(|b| b.parent_branch_id.as_deref())
        .collect();
    let created: Vec<i64> = branches.iter().map(|b| b.created_tx_id as i64).collect();
    let archived: Vec<i64> = branches
        .iter()
        .map(|b| if b.archived { 1 } else { 0 })
        .collect();
    b.add_sym_col("branch_id", &ids);
    b.add_sym_col("name", &names);
    b.add_sym_col_nullable("parent_branch_id", &parents);
    b.add_i64_col("created_tx_id", &created, None);
    b.add_i64_col("archived", &archived, None);

    b.finish()
}

pub fn load_branches(table: &RayObj) -> Result<Vec<Branch>> {
    let tbl = table.as_ptr();
    let nrows = unsafe { ffi::ray_table_nrows(tbl) };
    let ncols = unsafe { ffi::ray_table_ncols(tbl) };

    let ids = read_sym_col(tbl, 0, nrows)?;
    let names = read_sym_col(tbl, 1, nrows)?;
    let parents = read_sym_nullable_col(tbl, 2, nrows)?;
    let created = read_i64_col(tbl, 3, nrows);
    let archived_col = if ncols >= 5 {
        read_i64_col(tbl, 4, nrows)
    } else {
        vec![0i64; nrows as usize]
    };

    let mut branches = Vec::with_capacity(nrows as usize);
    for i in 0..nrows as usize {
        branches.push(Branch {
            branch_id: ids[i].clone(),
            name: names[i].clone(),
            parent_branch_id: parents[i].clone(),
            created_tx_id: created[i] as u64,
            archived: archived_col[i] != 0,
            claimed_by: None, // splay tables don't store this; JSONL is the source of truth
        });
    }
    Ok(branches)
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

pub fn data_dir() -> std::path::PathBuf {
    if let Ok(custom) = std::env::var("RAY_EXOMEM_HOME") {
        return std::path::PathBuf::from(custom);
    }
    dirs::home_dir()
        .expect("home dir")
        .join(".ray-exomem")
}

pub fn tree_root() -> std::path::PathBuf {
    data_dir().join("tree")
}

fn path_to_cstring(path: &Path) -> Result<CString> {
    CString::new(path.to_str().ok_or_else(|| anyhow!("non-UTF8 path"))?)
        .context("path contains null byte")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_root_follows_env() {
        std::env::set_var("RAY_EXOMEM_HOME", "/tmp/ray-exomem-test");
        assert_eq!(tree_root(), std::path::PathBuf::from("/tmp/ray-exomem-test/tree"));
        std::env::remove_var("RAY_EXOMEM_HOME");
    }
}
