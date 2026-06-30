//! Tagged-datom encoding — re-exported from the `ray-datom` crate.
//!
//! The implementation moved to `ray-datom::datom`. Existing in-tree call
//! sites continue to use `crate::datom::*` via this re-export so the
//! extraction is non-breaking. New code should depend on `ray-datom`
//! directly.

pub use ray_datom::datom::{encode_str, encode_sym, kind, payload, KIND_I64, KIND_STR, KIND_SYM};
