//! Tagged datom encoding — packs type information into int64 high bits.
//!
//! Layout:
//!   Bits 63-62 = 00 → I64 (plain integer, or negative)
//!   Bits 63-62 = 01 → SYM (symbol intern ID in low 61 bits)
//!   Bits 63-62 = 10 → STR (string intern ID in low 61 bits)
//!   Negative values are always I64 (bit 63 set).

const DATOM_TAG_SYM: i64 = 0x2000000000000000;
const DATOM_TAG_STR: i64 = 0x4000000000000000;
const DATOM_TAG_MASK: i64 = 0x6000000000000000;
const DATOM_PAYLOAD_MASK: i64 = 0x1FFFFFFFFFFFFFFF;

pub const KIND_I64: i32 = 0;
pub const KIND_SYM: i32 = 1;
pub const KIND_STR: i32 = 2;

pub fn encode_str(sym_id: i64) -> i64 {
    DATOM_TAG_STR | (sym_id & DATOM_PAYLOAD_MASK)
}

pub fn encode_sym(sym_id: i64) -> i64 {
    DATOM_TAG_SYM | (sym_id & DATOM_PAYLOAD_MASK)
}

pub fn kind(encoded: i64) -> i32 {
    if encoded < 0 {
        return KIND_I64;
    }
    match encoded & DATOM_TAG_MASK {
        DATOM_TAG_SYM => KIND_SYM,
        DATOM_TAG_STR => KIND_STR,
        _ => KIND_I64,
    }
}

pub fn payload(encoded: i64) -> i64 {
    encoded & DATOM_PAYLOAD_MASK
}
