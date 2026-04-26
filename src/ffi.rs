use libc::{c_char, c_int, c_void};

#[repr(C)]
pub struct ray_t {
    _private: [u8; 0],
}

#[repr(C)]
pub struct ray_runtime_t {
    _private: [u8; 0],
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[allow(non_camel_case_types)]
pub type ray_err_t = c_int;
pub const RAY_OK: ray_err_t = 0;
pub const RAY_ERR_CORRUPT: ray_err_t = 10;

// RAY_ERROR object type tag (see rayforce.h: `#define RAY_ERROR 127`).
pub const RAY_ERROR: i8 = 127;

// ---------------------------------------------------------------------------
// Type constants
// ---------------------------------------------------------------------------

pub const RAY_I64: i8 = 5;
pub const RAY_F32: i8 = 6;
pub const RAY_F64: i8 = 7;
pub const RAY_BOOL: i8 = 1;
pub const RAY_U8: i8 = 2;
pub const RAY_I16: i8 = 3;
pub const RAY_I32: i8 = 4;
pub const RAY_DATE: i8 = 8;
pub const RAY_TIME: i8 = 9;
pub const RAY_TIMESTAMP: i8 = 10;
pub const RAY_SYM: i8 = 12;
pub const RAY_STR: i8 = 13;
pub const RAY_TABLE: i8 = 98;
// SYM width constants
pub const RAY_SYM_W8: u8 = 0x00;
pub const RAY_SYM_W16: u8 = 0x01;
pub const RAY_SYM_W32: u8 = 0x02;

extern "C" {
    // -----------------------------------------------------------------------
    // Runtime
    // -----------------------------------------------------------------------

    pub fn ray_runtime_create(argc: c_int, argv: *mut *mut c_char) -> *mut ray_runtime_t;
    pub fn ray_runtime_create_with_sym(sym_path: *const c_char) -> *mut ray_runtime_t;
    pub fn ray_runtime_create_with_sym_err(
        sym_path: *const c_char,
        out_sym_err: *mut ray_err_t,
    ) -> *mut ray_runtime_t;
    pub fn ray_runtime_destroy(rt: *mut ray_runtime_t);

    // -----------------------------------------------------------------------
    // Eval / Format
    // -----------------------------------------------------------------------

    pub fn ray_eval_str(source: *const c_char) -> *mut ray_t;
    pub fn ray_fmt(obj: *mut ray_t, mode: c_int) -> *mut ray_t;
    pub fn ray_fmt_set_precision(digits: c_int);
    pub fn ray_fmt_set_width(cols: c_int);

    // -----------------------------------------------------------------------
    // String access
    // -----------------------------------------------------------------------

    pub fn ray_str_ptr(s: *mut ray_t) -> *const c_char;
    pub fn ray_str_len(s: *mut ray_t) -> usize;

    // -----------------------------------------------------------------------
    // Ref counting
    // -----------------------------------------------------------------------

    pub fn ray_retain(v: *mut ray_t);
    pub fn ray_release(v: *mut ray_t);

    // -----------------------------------------------------------------------
    // Introspection helpers
    // -----------------------------------------------------------------------
    // `ray_obj_type`, typed `ray_vec_get_*` — implemented in Rust below using the
    // public `ray_vec_get` API (upstream does not export the fork-only helpers).

    // -----------------------------------------------------------------------
    // Error handling
    // -----------------------------------------------------------------------

    pub fn ray_error_msg() -> *const c_char;
    pub fn ray_error_clear();

    // RAY_ERROR object inspection — read the 8-byte ASCII code packed into
    // the error object's `sdata`. `ray_err_from_obj` maps the code back to
    // the `ray_err_t` enum; `ray_err_code_str` is the inverse name table.
    pub fn ray_err_from_obj(err: *mut ray_t) -> ray_err_t;
    pub fn ray_err_code(err: *mut ray_t) -> *const c_char;
    pub fn ray_err_code_str(e: ray_err_t) -> *const c_char;

    // -----------------------------------------------------------------------
    // Environment API (promoted to public header in fork Feature C)
    // -----------------------------------------------------------------------

    pub fn ray_env_get(sym_id: i64) -> *mut ray_t;
    pub fn ray_env_set(sym_id: i64, val: *mut ray_t) -> ray_err_t;
    /// Release all global env values (paired with `ray_lang_init` to restore builtins).
    pub fn ray_env_destroy();
    /// Re-initialize env + builtins after `ray_env_destroy` (see `RayforceEngine::reconcile_lang_env`).
    pub fn ray_lang_init() -> ray_err_t;

    // -----------------------------------------------------------------------
    // Version
    // -----------------------------------------------------------------------

    pub fn ray_version_string() -> *const c_char;

    // -----------------------------------------------------------------------
    // Symbol table
    // -----------------------------------------------------------------------

    pub fn ray_sym_intern(s: *const c_char, len: usize) -> i64;
    pub fn ray_sym_find(s: *const c_char, len: usize) -> i64;
    pub fn ray_sym_str(id: i64) -> *mut ray_t;
    pub fn ray_sym_count() -> u32;
    pub fn ray_sym_save(path: *const c_char) -> ray_err_t;
    pub fn ray_sym_load(path: *const c_char) -> ray_err_t;

    // -----------------------------------------------------------------------
    // Table API
    // -----------------------------------------------------------------------

    pub fn ray_table_new(ncols: i64) -> *mut ray_t;
    pub fn ray_table_add_col(tbl: *mut ray_t, name_id: i64, col: *mut ray_t) -> *mut ray_t;
    pub fn ray_table_get_col(tbl: *mut ray_t, name_id: i64) -> *mut ray_t;
    pub fn ray_table_get_col_idx(tbl: *mut ray_t, idx: i64) -> *mut ray_t;
    pub fn ray_table_col_name(tbl: *mut ray_t, idx: i64) -> i64;
    pub fn ray_table_ncols(tbl: *mut ray_t) -> i64;
    pub fn ray_table_nrows(tbl: *mut ray_t) -> i64;

    // -----------------------------------------------------------------------
    // Vector API
    // -----------------------------------------------------------------------

    pub fn ray_vec_new(typ: i8, capacity: i64) -> *mut ray_t;
    pub fn ray_vec_append(vec: *mut ray_t, elem: *const c_void) -> *mut ray_t;
    pub fn ray_vec_get(vec: *mut ray_t, idx: i64) -> *mut c_void;
    pub fn ray_vec_set_null(vec: *mut ray_t, idx: i64, is_null: bool);
    pub fn ray_vec_is_null(vec: *mut ray_t, idx: i64) -> bool;

    // SYM vector (dictionary-encoded strings)
    pub fn ray_sym_vec_new(width: u8, capacity: i64) -> *mut ray_t;

    // STR vector (variable-length strings)
    pub fn ray_str_vec_append(vec: *mut ray_t, s: *const c_char, len: usize) -> *mut ray_t;
    pub fn ray_str_vec_get(vec: *mut ray_t, idx: i64, out_len: *mut usize) -> *const c_char;

    // -----------------------------------------------------------------------
    // Splayed table I/O
    // -----------------------------------------------------------------------

    pub fn ray_splay_save(
        tbl: *mut ray_t,
        dir: *const c_char,
        sym_path: *const c_char,
    ) -> ray_err_t;
    pub fn ray_splay_load(dir: *const c_char, sym_path: *const c_char) -> *mut ray_t;
    pub fn ray_read_splayed(dir: *const c_char, sym_path: *const c_char) -> *mut ray_t;

    // -----------------------------------------------------------------------
    // Column I/O
    // -----------------------------------------------------------------------

    pub fn ray_col_save(vec: *mut ray_t, path: *const c_char) -> ray_err_t;
    pub fn ray_col_load(path: *const c_char) -> *mut ray_t;
}

// ---------------------------------------------------------------------------
// Vector / type helpers (mirror fork `runtime.c` using public `ray_vec_get`)
// ---------------------------------------------------------------------------

/// `v->type` — offset matches `ray_t` in `include/rayforce.h`.
#[inline]
pub unsafe fn ray_obj_type(v: *mut ray_t) -> i8 {
    if v.is_null() {
        return 0;
    }
    *v.cast::<u8>().add(18).cast::<i8>()
}

#[inline]
pub unsafe fn ray_obj_attrs(v: *mut ray_t) -> u8 {
    if v.is_null() {
        return 0;
    }
    *v.cast::<u8>().add(19)
}

#[inline]
pub unsafe fn ray_vec_get_i64(vec: *mut ray_t, idx: i64) -> i64 {
    let p = ray_vec_get(vec, idx);
    if p.is_null() {
        return 0;
    }
    match ray_obj_type(vec) {
        RAY_I64 | RAY_DATE | RAY_TIME | RAY_TIMESTAMP => *(p as *const i64),
        RAY_I32 => *(p as *const i32) as i64,
        RAY_I16 => *(p as *const i16) as i64,
        RAY_U8 | RAY_BOOL => *(p as *const u8) as i64,
        _ => 0,
    }
}

#[inline]
pub unsafe fn ray_vec_get_f64(vec: *mut ray_t, idx: i64) -> f64 {
    let p = ray_vec_get(vec, idx);
    if p.is_null() {
        return 0.0;
    }
    match ray_obj_type(vec) {
        RAY_F64 => *(p as *const f64),
        RAY_F32 => *(p as *const f32) as f64,
        _ => 0.0,
    }
}

#[inline]
pub unsafe fn ray_vec_get_sym_id(vec: *mut ray_t, idx: i64) -> i64 {
    let p = ray_vec_get(vec, idx);
    if p.is_null() || ray_obj_type(vec) != RAY_SYM {
        return 0;
    }
    match ray_obj_attrs(vec) & 0x03 {
        0 => *(p as *const u8) as i64,
        1 => *(p as *const u16) as i64,
        2 => *(p as *const u32) as i64,
        3 => *(p as *const i64),
        _ => 0,
    }
}
