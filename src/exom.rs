//! Exom directory manager — manages the on-disk layout for multi-exom persistence.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::storage;

fn is_rayfall_symbol_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(ch, '_' | '.' | '-' | '!' | '?' | '+' | '*' | '/' | '%' | '<' | '>' | '=' | '&' | '|')
    })
}

/// Manages the `~/.ray-exomem/` directory structure.
pub struct ExomDir {
    root: PathBuf,
    /// True when the sym file was corrupt/incompatible and we must recover
    /// exom state from JSONL sidecars instead of splay tables.
    recovery_mode: bool,
}

impl ExomDir {
    /// Open (or create) the data directory. Loads the global symbol table.
    /// If the symbol table is corrupt or incompatible (e.g. after a binary
    /// upgrade), enters recovery mode — exoms will be loaded from JSONL
    /// sidecars and splay tables rebuilt.
    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("exoms"))
            .with_context(|| format!("failed to create {}/exoms", root.display()))?;

        let mut dir = Self { root, recovery_mode: false };

        if !storage::sym_load(&dir.sym_path())? {
            eprintln!(
                "[ray-exomem] WARNING: symbol table incompatible (binary upgrade?). \
                 Recovering from JSONL sidecars: {}",
                dir.root.display()
            );
            dir.wipe_sym()?;
            dir.recovery_mode = true;
        }

        Ok(dir)
    }

    /// Whether we're in recovery mode (sym was corrupt, load from JSONL).
    pub fn is_recovery_mode(&self) -> bool {
        self.recovery_mode
    }

    /// Remove only the sym files so they can be regenerated from fresh data.
    fn wipe_sym(&self) -> Result<()> {
        let sym = self.sym_path();
        if sym.exists() {
            std::fs::remove_file(&sym)
                .with_context(|| format!("failed to remove {}", sym.display()))?;
        }
        let sym_lk = self.root.join("sym.lk");
        if sym_lk.exists() {
            let _ = std::fs::remove_file(&sym_lk);
        }
        Ok(())
    }

    /// Path to the global symbol table file.
    pub fn sym_path(&self) -> PathBuf {
        self.root.join("sym")
    }

    /// Path to a specific exom's directory.
    pub fn exom_path(&self, name: &str) -> PathBuf {
        self.root.join("exoms").join(name)
    }

    /// Path to a specific table within an exom.
    pub fn table_path(&self, exom: &str, table: &str) -> PathBuf {
        self.exom_path(exom).join(table)
    }

    /// List all exom names (directories under exoms/).
    pub fn list_exoms(&self) -> Result<Vec<String>> {
        let exoms_dir = self.root.join("exoms");
        if !exoms_dir.exists() {
            return Ok(Vec::new());
        }

        let mut names = Vec::new();
        for entry in std::fs::read_dir(&exoms_dir)
            .with_context(|| format!("failed to read {}", exoms_dir.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// Create a new exom directory.
    pub fn create_exom(&self, name: &str) -> Result<()> {
        if !is_rayfall_symbol_name(name) {
            bail!("exom '{}' is not a valid bare Rayfall symbol", name);
        }
        let path = self.exom_path(name);
        if path.exists() {
            bail!("exom '{}' already exists", name);
        }
        std::fs::create_dir_all(&path)
            .with_context(|| format!("failed to create exom dir {}", path.display()))?;
        Ok(())
    }

    /// Delete an exom directory and all its data.
    pub fn delete_exom(&self, name: &str) -> Result<()> {
        let path = self.exom_path(name);
        if !path.exists() {
            bail!("exom '{}' does not exist", name);
        }
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("failed to delete exom dir {}", path.display()))?;
        Ok(())
    }

    /// Rename an exom directory.
    pub fn rename_exom(&self, old: &str, new: &str) -> Result<()> {
        if !is_rayfall_symbol_name(new) {
            bail!("exom '{}' is not a valid bare Rayfall symbol", new);
        }
        let old_path = self.exom_path(old);
        let new_path = self.exom_path(new);
        if !old_path.exists() {
            bail!("exom '{}' does not exist", old);
        }
        if new_path.exists() {
            bail!("exom '{}' already exists", new);
        }
        std::fs::rename(&old_path, &new_path)
            .with_context(|| format!("failed to rename {} -> {}", old, new))?;
        Ok(())
    }
}
