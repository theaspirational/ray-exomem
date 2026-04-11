use crate::exom::{self, ExomKind, ExomMeta};
use crate::path::{ensure_not_reserved_as_exom, TreePath};
use crate::tree::{classify, NodeKind};
use std::io;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("path: {0}")] Path(#[from] crate::path::PathError),
    #[error("io: {0}")] Io(#[from] io::Error),
    #[error("cannot nest inside exom at {0}")] NestInsideExom(String),
    #[error("already exists as {0:?} at {1}")] AlreadyExistsDifferent(NodeKind, String),
}

pub fn init_project(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    crate::tree::check_no_exom_ancestor(tree_root, path)
        .map_err(ScaffoldError::NestInsideExom)?;
    let leaf = path.to_disk_path(tree_root);
    std::fs::create_dir_all(&leaf)?;

    // main exom
    let main_path = leaf.join("main");
    match classify(&main_path) {
        NodeKind::Missing => {
            exom::write_meta(&main_path, &ExomMeta::new_project_main())?;
        }
        NodeKind::Exom => {
            let meta = exom::read_meta(&main_path)?;
            if meta.kind != ExomKind::ProjectMain {
                return Err(ScaffoldError::AlreadyExistsDifferent(NodeKind::Exom, main_path.display().to_string()));
            }
        }
        NodeKind::Folder => {
            return Err(ScaffoldError::AlreadyExistsDifferent(NodeKind::Folder, main_path.display().to_string()));
        }
    }

    // sessions/ folder (empty dir, no metadata)
    let sessions_path = leaf.join("sessions");
    std::fs::create_dir_all(&sessions_path)?;
    Ok(())
}

pub fn new_bare_exom(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    if let Some(last) = path.last() {
        ensure_not_reserved_as_exom(last)?;
    }
    crate::tree::check_no_exom_ancestor(tree_root, path)
        .map_err(ScaffoldError::NestInsideExom)?;
    let disk = path.to_disk_path(tree_root);
    match classify(&disk) {
        NodeKind::Missing => {
            std::fs::create_dir_all(&disk)?;
            exom::write_meta(&disk, &ExomMeta::new_bare())?;
            Ok(())
        }
        NodeKind::Exom => Ok(()), // idempotent
        NodeKind::Folder => Err(ScaffoldError::AlreadyExistsDifferent(NodeKind::Folder, disk.display().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn tp(s: &str) -> TreePath { s.parse().unwrap() }

    #[test]
    fn init_creates_main_and_sessions() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::ath::lynx::orsl")).unwrap();
        assert_eq!(classify(&d.path().join("work/ath/lynx/orsl/main")), NodeKind::Exom);
        assert_eq!(classify(&d.path().join("work/ath/lynx/orsl/sessions")), NodeKind::Folder);
    }

    #[test]
    fn init_is_idempotent() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::ath")).unwrap();
        init_project(d.path(), &tp("work::ath")).unwrap();
    }

    #[test]
    fn projects_nest_freely() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::ath::lynx::orsl")).unwrap();
        init_project(d.path(), &tp("work::ath")).unwrap();
        assert_eq!(classify(&d.path().join("work/ath/main")), NodeKind::Exom);
        assert_eq!(classify(&d.path().join("work/ath/lynx/orsl/main")), NodeKind::Exom);
    }

    #[test]
    fn cannot_nest_inside_exom() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work")).unwrap(); // work/main is exom
        let err = init_project(d.path(), &tp("work::main::deeper"));
        assert!(matches!(err, Err(ScaffoldError::NestInsideExom(_))));
    }

    #[test]
    fn new_bare_exom_rejects_reserved() {
        let d = tempdir().unwrap();
        assert!(matches!(new_bare_exom(d.path(), &tp("work::sessions")),
                         Err(ScaffoldError::Path(_))));
    }

    #[test]
    fn new_bare_exom_is_idempotent() {
        let d = tempdir().unwrap();
        new_bare_exom(d.path(), &tp("work::ath::notes")).unwrap();
        new_bare_exom(d.path(), &tp("work::ath::notes")).unwrap();
    }
}
