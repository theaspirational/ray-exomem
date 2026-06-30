//! Typed fact values — re-exported from the `ray-datom` crate.
//!
//! The implementation moved to `ray-datom::fact_value`. Existing in-tree
//! call sites continue to use `crate::fact_value::*` via this re-export so
//! the extraction is non-breaking. New code should depend on `ray-datom`
//! directly.

pub use ray_datom::fact_value::{FactValue, SymValue};
