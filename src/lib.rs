pub mod agent_guide;
pub mod backend;
pub mod brain;
pub mod http_error;
pub mod client;
pub mod context;
pub mod datom;
pub mod exom;
pub mod ffi;
pub mod path;
pub mod rayfall_ast;
pub mod rayfall_parser;
pub mod rules;
pub mod scaffold;
pub mod storage;
pub mod system_schema;
pub mod tree;
pub mod web;

use anyhow::{Context, Result};
use std::{fs, path::Path};

pub use backend::{rayforce_version, RayforceEngine};

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

pub fn backend_name() -> &'static str {
    "rayforce2"
}

pub fn frontend_name() -> &'static str {
    "ray-exomem"
}

pub fn frontend_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn syntax_name() -> &'static str {
    "rayfall-native"
}

pub fn build_git_sha() -> &'static str {
    option_env!("RAY_EXOMEM_GIT_SHA").unwrap_or("unknown")
}

pub fn build_unix_timestamp() -> &'static str {
    option_env!("RAY_EXOMEM_BUILD_UNIX").unwrap_or("0")
}

pub fn build_identity() -> String {
    format!(
        "{}+{}-{}",
        frontend_version(),
        build_git_sha(),
        build_unix_timestamp()
    )
}

// ---------------------------------------------------------------------------
// Core execution — thin wrappers over native rayforce2
// ---------------------------------------------------------------------------

pub fn run_source(source: &str) -> Result<String> {
    let engine = RayforceEngine::new()?;
    engine.eval(source)
}

pub fn run_file(path: &Path) -> Result<String> {
    if path.extension().and_then(|ext| ext.to_str()) == Some("dl") {
        anyhow::bail!(
            "legacy Teide .dl files are not supported by ray-exomem; use Rayfall .ray source instead"
        );
    }

    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read source file: {}", path.display()))?;
    run_source(&source)
}

// ---------------------------------------------------------------------------
// Migration guardrail: this module must never contain Teide parsing,
// Teide AST construction, or Teide-to-Rayfall translation code.
//
// The test suite includes compile-time and runtime checks that verify
// no Teide translation path exists in the frontend.
// ---------------------------------------------------------------------------

/// Global test lock — rayforce2 uses global state, so tests must be serialized.
#[cfg(test)]
pub fn global_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn test_lock() -> &'static Mutex<()> {
        crate::global_test_lock()
    }

    // -----------------------------------------------------------------------
    // Smoke tests — native Rayfall execution
    // -----------------------------------------------------------------------

    #[test]
    fn arithmetic_eval() {
        let _guard = test_lock().lock().unwrap();
        let output = run_source("(+ 1 2)").unwrap();
        assert!(output.trim() == "3", "expected 3, got: {}", output.trim());
    }

    #[test]
    fn malformed_source_returns_error() {
        let _guard = test_lock().lock().unwrap();
        let err = run_source("(+ 1").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("error") || text.contains("parse"),
            "unexpected malformed-source error: {}",
            text
        );
    }

    #[test]
    fn legacy_dl_inputs_are_rejected() {
        let _guard = test_lock().lock().unwrap();
        let filename = format!("ray-exomem-legacy-{}.dl", std::process::id());
        let path = std::env::temp_dir().join(filename);
        std::fs::write(&path, "(+ 1 2)").unwrap();

        let err = run_file(&path).unwrap_err();
        let err_text = err.to_string();
        assert!(
            err_text.contains("legacy Teide .dl files are not supported"),
            "unexpected error: {}",
            err
        );

        let _ = std::fs::remove_file(&path);
    }

    // -----------------------------------------------------------------------
    // Migration guardrails — no Teide translation path
    // -----------------------------------------------------------------------

    #[test]
    fn no_teide_parser_in_src() {
        // Scan all .rs files in src/ for references to Teide parsing/translation.
        // This is a runtime guardrail that prevents accidental reintroduction.
        //
        // Patterns are split so the test file itself does not trigger a match.
        let prefixes = ["teide_", "Teide"];
        let suffixes = [
            "parse",
            "ast",
            "Parser",
            "Ast",
            "to_rayfall",
            "translate",
            "rewrite",
            "Translator",
        ];
        for entry in std::fs::read_dir("src").unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            // Skip the test file itself (this file) to avoid self-detection.
            if path.file_name().map(|n| n == "lib.rs").unwrap_or(false) {
                continue;
            }
            let contents = std::fs::read_to_string(&path).unwrap();
            for prefix in &prefixes {
                for suffix in &suffixes {
                    let pattern = format!("{}{}", prefix, suffix);
                    assert!(
                        !contents.contains(&pattern),
                        "found forbidden Teide reference '{}' in {}",
                        pattern,
                        path.display()
                    );
                }
            }
        }
    }

    #[test]
    fn no_teide_dependency_in_cargo_toml() {
        let toml = std::fs::read_to_string("Cargo.toml").unwrap();
        assert!(
            !toml.contains("teide-parser"),
            "Cargo.toml must not depend on teide-parser"
        );
        assert!(
            !toml.contains("teide-ast"),
            "Cargo.toml must not depend on teide-ast"
        );
    }

    #[test]
    fn version_info_is_consistent() {
        assert_eq!(backend_name(), "rayforce2");
        assert_eq!(frontend_name(), "ray-exomem");
        assert_eq!(frontend_version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(syntax_name(), "rayfall-native");
        assert!(!build_git_sha().is_empty(), "git sha should not be empty");
        assert!(
            build_unix_timestamp().parse::<u64>().is_ok(),
            "build timestamp should be numeric"
        );
        assert!(
            build_identity().starts_with(frontend_version()),
            "build identity should begin with the crate version"
        );

        let ver = rayforce_version();
        assert!(
            !ver.is_empty() && ver != "unknown",
            "rayforce version should be available"
        );
    }
}
