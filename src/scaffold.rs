use crate::exom::{self, ExomKind, ExomMeta};
use crate::path::{ensure_not_reserved_as_exom, TreePath};
use crate::tree::{classify, NodeKind};
use std::io;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("path: {0}")]
    Path(#[from] crate::path::PathError),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("cannot nest inside exom at {0}")]
    NestInsideExom(String),
    #[error("already exists as {0:?} at {1}")]
    AlreadyExistsDifferent(NodeKind, String),
    #[error("not found at {0}")]
    NotFound(String),
}

pub fn init_project(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    crate::tree::check_no_exom_ancestor(tree_root, path).map_err(ScaffoldError::NestInsideExom)?;
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
                return Err(ScaffoldError::AlreadyExistsDifferent(
                    NodeKind::Exom,
                    main_path.display().to_string(),
                ));
            }
        }
        NodeKind::Folder => {
            return Err(ScaffoldError::AlreadyExistsDifferent(
                NodeKind::Folder,
                main_path.display().to_string(),
            ));
        }
    }

    // sessions/ folder (empty dir, no metadata)
    let sessions_path = leaf.join("sessions");
    std::fs::create_dir_all(&sessions_path)?;
    Ok(())
}

/// Create an empty folder at `path` (and any missing parents) without
/// scaffolding `main` or `sessions/`. Idempotent: re-running is a no-op
/// when the folder already exists. Refuses to create anything inside an
/// existing exom and refuses to overwrite an existing exom at this path.
pub fn new_folder(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    crate::tree::check_no_exom_ancestor(tree_root, path).map_err(ScaffoldError::NestInsideExom)?;
    let disk = path.to_disk_path(tree_root);
    match classify(&disk) {
        NodeKind::Missing | NodeKind::Folder => {
            std::fs::create_dir_all(&disk)?;
            Ok(())
        }
        NodeKind::Exom => Err(ScaffoldError::AlreadyExistsDifferent(
            NodeKind::Exom,
            disk.display().to_string(),
        )),
    }
}

/// Walk the disk subtree rooted at `path` and return the slash-form paths
/// of every exom (a directory containing `exom.json`) inside, including
/// `path` itself when it is an exom. Used pre-deletion so callers can
/// evict in-memory `ExomState` entries and unbind them from the engine
/// before the directory is removed.
pub fn collect_exoms_under(
    tree_root: &Path,
    path: &TreePath,
) -> Result<Vec<String>, ScaffoldError> {
    let root_disk = path.to_disk_path(tree_root);
    let path_slash = path.to_slash_string();
    let mut out = Vec::new();
    match classify(&root_disk) {
        NodeKind::Missing => Err(ScaffoldError::NotFound(root_disk.display().to_string())),
        NodeKind::Exom => {
            out.push(path_slash);
            Ok(out)
        }
        NodeKind::Folder => {
            walk_collect_exoms(&root_disk, &path_slash, &mut out)?;
            Ok(out)
        }
    }
}

fn walk_collect_exoms(disk: &Path, slash: &str, out: &mut Vec<String>) -> io::Result<()> {
    for entry in std::fs::read_dir(disk)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_slash = if slash.is_empty() {
            name
        } else {
            format!("{slash}/{name}")
        };
        match classify(&p) {
            NodeKind::Exom => out.push(child_slash),
            NodeKind::Folder => walk_collect_exoms(&p, &child_slash, out)?,
            NodeKind::Missing => {}
        }
    }
    Ok(())
}

/// Recursively delete the disk subtree at `path`. Returns `NotFound` when
/// nothing exists at the target. Caller is responsible for evicting any
/// in-memory exom state and updating downstream stores (auth shares,
/// engine bindings) — this function only touches the filesystem.
pub fn delete_subtree(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    let disk = path.to_disk_path(tree_root);
    if !disk.exists() {
        return Err(ScaffoldError::NotFound(disk.display().to_string()));
    }
    std::fs::remove_dir_all(&disk)?;
    Ok(())
}

pub fn new_bare_exom(tree_root: &Path, path: &TreePath) -> Result<(), ScaffoldError> {
    if let Some(last) = path.last() {
        ensure_not_reserved_as_exom(last)?;
    }
    crate::tree::check_no_exom_ancestor(tree_root, path).map_err(ScaffoldError::NestInsideExom)?;
    let disk = path.to_disk_path(tree_root);
    match classify(&disk) {
        NodeKind::Missing => {
            std::fs::create_dir_all(&disk)?;
            exom::write_meta(&disk, &ExomMeta::new_bare())?;
            Ok(())
        }
        NodeKind::Exom => Ok(()), // idempotent
        NodeKind::Folder => Err(ScaffoldError::AlreadyExistsDifferent(
            NodeKind::Folder,
            disk.display().to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn tp(s: &str) -> TreePath {
        s.parse().unwrap()
    }

    #[test]
    fn init_creates_main_and_sessions() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::team::project::repo")).unwrap();
        assert_eq!(
            classify(&d.path().join("work/team/project/repo/main")),
            NodeKind::Exom
        );
        assert_eq!(
            classify(&d.path().join("work/team/project/repo/sessions")),
            NodeKind::Folder
        );
    }

    #[test]
    fn init_is_idempotent() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::team")).unwrap();
        init_project(d.path(), &tp("work::team")).unwrap();
    }

    #[test]
    fn projects_nest_freely() {
        let d = tempdir().unwrap();
        init_project(d.path(), &tp("work::team::project::repo")).unwrap();
        init_project(d.path(), &tp("work::team")).unwrap();
        assert_eq!(classify(&d.path().join("work/team/main")), NodeKind::Exom);
        assert_eq!(
            classify(&d.path().join("work/team/project/repo/main")),
            NodeKind::Exom
        );
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
        assert!(matches!(
            new_bare_exom(d.path(), &tp("work::sessions")),
            Err(ScaffoldError::Path(_))
        ));
    }

    #[test]
    fn new_bare_exom_is_idempotent() {
        let d = tempdir().unwrap();
        new_bare_exom(d.path(), &tp("work::ath::notes")).unwrap();
        new_bare_exom(d.path(), &tp("work::ath::notes")).unwrap();
    }
}
