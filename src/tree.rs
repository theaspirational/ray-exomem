use crate::exom::{self, ExomKind};
use crate::path::TreePath;
use serde::Serialize;
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

// ---------------------------------------------------------------------------
// Tree walker
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TreeNode {
    Folder {
        name: String,
        path: String, // slash form
        children: Vec<TreeNode>,
    },
    Exom {
        name: String,
        path: String,
        exom_kind: ExomKind,
        fact_count: u64,
        current_branch: String,
        last_tx: Option<String>,
        branches: Option<Vec<String>>, // only when requested
        archived: bool,
        closed: bool,
        session: Option<exom::SessionMeta>,
    },
}

pub struct WalkOptions {
    pub depth: Option<usize>,
    pub include_archived: bool,
    pub include_branches: bool,
    pub include_activity: bool,
}

pub fn walk(tree_root: &std::path::Path, start: &crate::path::TreePath, opts: &WalkOptions)
    -> std::io::Result<TreeNode>
{
    let start_disk = start.to_disk_path(tree_root);
    walk_inner(&start_disk, start, 0, opts)
}

fn walk_inner(
    disk: &std::path::Path,
    path: &crate::path::TreePath,
    depth: usize,
    opts: &WalkOptions,
) -> std::io::Result<TreeNode> {
    let name = path.last().unwrap_or("").to_string();
    let slash = path.to_slash_string();
    match classify(disk) {
        NodeKind::Missing => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
        NodeKind::Exom => {
            let meta = exom::read_meta(disk)?;
            let archived = meta.session.as_ref().and_then(|s| s.archived_at.as_ref()).is_some();
            let closed = meta.session.as_ref().and_then(|s| s.closed_at.as_ref()).is_some();
            if archived && !opts.include_archived {
                return Ok(TreeNode::Folder { name, path: slash, children: vec![] });
            }
            // fact_count / last_tx / branches are pulled from splay tables via callback in later tasks;
            // stubbed to zero/None here.
            Ok(TreeNode::Exom {
                name, path: slash,
                exom_kind: meta.kind,
                fact_count: 0,
                current_branch: meta.current_branch,
                last_tx: None,
                branches: if opts.include_branches { Some(vec![]) } else { None },
                archived, closed,
                session: meta.session,
            })
        }
        NodeKind::Folder => {
            let stop = matches!(opts.depth, Some(max) if depth >= max);
            let mut children = vec![];
            if !stop {
                let mut entries: Vec<_> = std::fs::read_dir(disk)?
                    .filter_map(|e| e.ok())
                    .collect();
                entries.sort_by_key(|e| e.file_name());
                for entry in entries {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let sub_path = path.join(&name).map_err(|e|
                        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
                    let sub_disk = entry.path();
                    if sub_disk.is_dir() {
                        children.push(walk_inner(&sub_disk, &sub_path, depth + 1, opts)?);
                    }
                }
            }
            Ok(TreeNode::Folder { name, path: slash, children })
        }
    }
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

    #[test]
    fn walks_a_scaffolded_project() {
        let d = tempdir().unwrap();
        crate::scaffold::init_project(d.path(), &"work::ath::lynx::orsl".parse().unwrap()).unwrap();
        let root: crate::path::TreePath = "work".parse().unwrap();
        let node = walk(d.path(), &root, &WalkOptions {
            depth: Some(5),
            include_archived: false,
            include_branches: false,
            include_activity: false,
        }).unwrap();
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("\"kind\":\"folder\""));
        assert!(json.contains("\"name\":\"main\""));
    }
}
