use crate::path::TreePath;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Missing,
    Folder,
    Exom,
}

pub fn classify(disk_path: &Path) -> NodeKind {
    if !disk_path.exists() {
        return NodeKind::Missing;
    }
    if disk_path.join("exom.json").exists() {
        NodeKind::Exom
    } else {
        NodeKind::Folder
    }
}

pub fn ensure_folder_path(tree_root: &Path, path: &TreePath) -> std::io::Result<PathBuf> {
    let disk = path.to_disk_path(tree_root);
    std::fs::create_dir_all(&disk)?;
    Ok(disk)
}

pub fn check_no_exom_ancestor(tree_root: &Path, path: &TreePath) -> Result<(), String> {
    let mut disk = tree_root.to_path_buf();
    for seg in path.segments() {
        disk.push(seg);
        if classify(&disk) == NodeKind::Exom && Some(seg.as_str()) != path.last() {
            return Err(format!("cannot nest inside exom {}", disk.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_is_missing() {
        let d = tempdir().unwrap();
        assert_eq!(classify(&d.path().join("nope")), NodeKind::Missing);
    }

    #[test]
    fn empty_dir_is_folder() {
        let d = tempdir().unwrap();
        fs::create_dir(d.path().join("f")).unwrap();
        assert_eq!(classify(&d.path().join("f")), NodeKind::Folder);
    }

    #[test]
    fn dir_with_exom_json_is_exom() {
        let d = tempdir().unwrap();
        fs::create_dir(d.path().join("e")).unwrap();
        fs::write(d.path().join("e/exom.json"), "{}").unwrap();
        assert_eq!(classify(&d.path().join("e")), NodeKind::Exom);
    }

    #[test]
    fn ancestor_exom_blocks_nesting() {
        let d = tempdir().unwrap();
        fs::create_dir_all(d.path().join("work/ath")).unwrap();
        fs::write(d.path().join("work/ath/exom.json"), "{}").unwrap();
        let p: TreePath = "work::ath::lynx".parse().unwrap();
        assert!(check_no_exom_ancestor(d.path(), &p).is_err());
    }

    #[test]
    fn leaf_exom_is_fine() {
        let d = tempdir().unwrap();
        fs::create_dir_all(d.path().join("work/main")).unwrap();
        fs::write(d.path().join("work/main/exom.json"), "{}").unwrap();
        let p: TreePath = "work::main".parse().unwrap();
        assert!(check_no_exom_ancestor(d.path(), &p).is_ok());
    }
}
