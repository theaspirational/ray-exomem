use crate::exom::{self, ExomKind};
use crate::path::TreePath;
use serde::Serialize;
use std::collections::BTreeSet;
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

#[derive(Debug, Serialize, Clone)]
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

pub fn walk(
    tree_root: &std::path::Path,
    sym_path: &std::path::Path,
    start: &crate::path::TreePath,
    opts: &WalkOptions,
) -> std::io::Result<TreeNode> {
    let start_disk = start.to_disk_path(tree_root);
    walk_inner(&start_disk, sym_path, start, 0, opts)
}

fn walk_inner(
    disk: &std::path::Path,
    sym_path: &std::path::Path,
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
            let archived = meta
                .session
                .as_ref()
                .and_then(|s| s.archived_at.as_ref())
                .is_some();
            let closed = meta
                .session
                .as_ref()
                .and_then(|s| s.closed_at.as_ref())
                .is_some();
            if archived && !opts.include_archived {
                return Ok(TreeNode::Folder {
                    name,
                    path: slash,
                    children: vec![],
                });
            }
            let stats = crate::brain::read_exom_stats(disk, sym_path).unwrap_or(
                crate::brain::ExomStats {
                    fact_count: 0,
                    last_tx: None,
                    branches: vec![],
                },
            );
            Ok(TreeNode::Exom {
                name,
                path: slash,
                exom_kind: meta.kind,
                fact_count: stats.fact_count,
                current_branch: meta.current_branch,
                last_tx: stats.last_tx,
                branches: if opts.include_branches {
                    Some(stats.branches)
                } else {
                    None
                },
                archived,
                closed,
                session: meta.session,
            })
        }
        NodeKind::Folder => {
            let stop = matches!(opts.depth, Some(max) if depth >= max);
            let mut children = vec![];
            if !stop {
                let mut entries: Vec<_> = std::fs::read_dir(disk)?.filter_map(|e| e.ok()).collect();
                entries.sort_by_key(|e| e.file_name());
                for entry in entries {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let sub_path = path.join(&name).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                    })?;
                    let sub_disk = entry.path();
                    if sub_disk.is_dir() {
                        children.push(walk_inner(
                            &sub_disk,
                            sym_path,
                            &sub_path,
                            depth + 1,
                            opts,
                        )?);
                    }
                }
            }
            Ok(TreeNode::Folder {
                name,
                path: slash,
                children,
            })
        }
    }
}

/// Walk the top-level entries under `tree_root`. Returns a synthetic Folder node with
/// name="" and path="" whose children are all top-level directories.
pub fn walk_root(
    tree_root: &std::path::Path,
    sym_path: &std::path::Path,
    opts: &WalkOptions,
) -> std::io::Result<TreeNode> {
    use std::fs;
    let mut children = vec![];
    if tree_root.exists() {
        let mut entries: Vec<_> = fs::read_dir(tree_root)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "_system" {
                continue;
            }
            if entry.path().is_dir() {
                let p: crate::path::TreePath =
                    name.parse().map_err(|e: crate::path::PathError| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                    })?;
                children.push(walk(tree_root, sym_path, &p, opts)?);
            }
        }
    }
    Ok(TreeNode::Folder {
        name: String::new(),
        path: String::new(),
        children,
    })
}

fn tree_path_starts_with(path: &TreePath, prefix: &TreePath) -> bool {
    path.len() >= prefix.len()
        && path
            .segments()
            .iter()
            .zip(prefix.segments().iter())
            .all(|(a, b)| a == b)
}

fn folder_name(path: &TreePath) -> String {
    path.last().unwrap_or("").to_string()
}

pub fn empty_folder(path: &TreePath) -> TreeNode {
    TreeNode::Folder {
        name: folder_name(path),
        path: path.to_slash_string(),
        children: vec![],
    }
}

pub fn walk_or_empty(
    tree_root: &std::path::Path,
    sym_path: &std::path::Path,
    path: &crate::path::TreePath,
    opts: &WalkOptions,
) -> std::io::Result<TreeNode> {
    let disk = path.to_disk_path(tree_root);
    if !disk.exists() {
        return Ok(empty_folder(path));
    }
    walk(tree_root, sym_path, path, opts)
}

/// Build a synthetic folder tree for a shared view.
///
/// If `requested` is inside a shared subtree, this returns the full on-disk walk for that path.
/// If `requested` is only an ancestor of shared paths, it returns a synthetic folder that reveals
/// just the descendants needed to reach the shared paths, without leaking sibling nodes.
pub fn walk_shared_projection(
    tree_root: &std::path::Path,
    sym_path: &std::path::Path,
    requested: &crate::path::TreePath,
    shared_paths: &[crate::path::TreePath],
    opts: &WalkOptions,
) -> std::io::Result<TreeNode> {
    if shared_paths
        .iter()
        .any(|grant| tree_path_starts_with(requested, grant))
    {
        return walk_or_empty(tree_root, sym_path, requested, opts);
    }

    let descendants: Vec<_> = shared_paths
        .iter()
        .filter(|grant| tree_path_starts_with(grant, requested))
        .collect();

    if descendants.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("no shared paths under {}", requested.to_slash_string()),
        ));
    }

    let mut child_segments = BTreeSet::new();
    for grant in descendants {
        if let Some(seg) = grant.segments().get(requested.len()) {
            child_segments.insert(seg.clone());
        }
    }

    let mut children = Vec::new();
    for seg in child_segments {
        let child_path = requested
            .join(&seg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let child_is_fully_shared = shared_paths
            .iter()
            .any(|grant| tree_path_starts_with(&child_path, grant));
        let child = if child_is_fully_shared {
            walk_or_empty(tree_root, sym_path, &child_path, opts)?
        } else {
            walk_shared_projection(tree_root, sym_path, &child_path, shared_paths, opts)?
        };
        children.push(child);
    }

    Ok(TreeNode::Folder {
        name: folder_name(requested),
        path: requested.to_slash_string(),
        children,
    })
}

/// Rename the last segment of `path` to `new_segment`. Returns the new `TreePath`.
/// Rejects session exom ids (callers should check meta.kind == Session before calling).
pub fn rename_last_segment(
    tree_root: &std::path::Path,
    path: &crate::path::TreePath,
    new_segment: &str,
) -> Result<crate::path::TreePath, String> {
    crate::path::validate_segment(new_segment).map_err(|e| e.to_string())?;
    let parent = path.parent().unwrap_or_else(crate::path::TreePath::root);
    let src = path.to_disk_path(tree_root);
    let parent_disk = if parent.is_empty() {
        tree_root.to_path_buf()
    } else {
        parent.to_disk_path(tree_root)
    };
    let dst = parent_disk.join(new_segment);

    // Same-name same-case rename is a no-op; allow it.
    if src == dst {
        return if parent.is_empty() {
            new_segment
                .parse()
                .map_err(|e: crate::path::PathError| e.to_string())
        } else {
            parent.join(new_segment).map_err(|e| e.to_string())
        };
    }

    if dst.exists() {
        return Err(format!("target already exists: {}", dst.display()));
    }

    // Case-insensitive collision check (necessary on macOS APFS).
    let current_last = path.last().unwrap_or("");
    if let Ok(entries) = std::fs::read_dir(&parent_disk) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str() {
                if name_str.eq_ignore_ascii_case(new_segment) && name_str != current_last {
                    return Err(format!(
                        "target already exists (case-insensitive): {}",
                        entry.path().display()
                    ));
                }
            }
        }
    }

    std::fs::rename(&src, &dst).map_err(|e| e.to_string())?;
    if parent.is_empty() {
        new_segment
            .parse()
            .map_err(|e: crate::path::PathError| e.to_string())
    } else {
        parent.join(new_segment).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn collect_paths(node: &TreeNode, out: &mut Vec<String>) {
        match node {
            TreeNode::Folder { path, children, .. } => {
                if !path.is_empty() {
                    out.push(path.clone());
                }
                for child in children {
                    collect_paths(child, out);
                }
            }
            TreeNode::Exom { path, .. } => out.push(path.clone()),
        }
    }

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
        fs::create_dir_all(d.path().join("work/team")).unwrap();
        fs::write(d.path().join("work/team/exom.json"), "{}").unwrap();
        let p: TreePath = "work::team::project".parse().unwrap();
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
        let sym = d.path().join("sym");
        crate::scaffold::init_project(d.path(), &"work::team::project::repo".parse().unwrap()).unwrap();
        let root: crate::path::TreePath = "work".parse().unwrap();
        let node = walk(
            d.path(),
            &sym,
            &root,
            &WalkOptions {
                depth: Some(5),
                include_archived: false,
                include_branches: false,
                include_activity: false,
            },
        )
        .unwrap();
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("\"kind\":\"folder\""));
        assert!(json.contains("\"name\":\"main\""));
    }

    #[test]
    fn walk_or_empty_returns_empty_folder_for_missing_namespace() {
        let d = tempdir().unwrap();
        let sym = d.path().join("sym");
        let path: TreePath = "alice@co.com".parse().unwrap();
        let node = walk_or_empty(
            d.path(),
            &sym,
            &path,
            &WalkOptions {
                depth: Some(5),
                include_archived: false,
                include_branches: false,
                include_activity: false,
            },
        )
        .unwrap();

        match node {
            TreeNode::Folder {
                name,
                path,
                children,
            } => {
                assert_eq!(name, "alice@co.com");
                assert_eq!(path, "alice@co.com");
                assert!(children.is_empty());
            }
            other => panic!("expected folder, got {:?}", other),
        }
    }

    #[test]
    fn shared_projection_reveals_only_the_ancestor_chain_for_deep_share() {
        let d = tempdir().unwrap();
        let sym = d.path().join("sym");
        fs::create_dir_all(d.path().join("alice/shared/docs/topic")).unwrap();
        fs::create_dir_all(d.path().join("alice/private/secret")).unwrap();

        let requested: TreePath = "alice".parse().unwrap();
        let shared_paths = vec!["alice/shared/docs".parse().unwrap()];
        let node = walk_shared_projection(
            d.path(),
            &sym,
            &requested,
            &shared_paths,
            &WalkOptions {
                depth: Some(8),
                include_archived: false,
                include_branches: false,
                include_activity: false,
            },
        )
        .unwrap();

        let mut paths = Vec::new();
        collect_paths(&node, &mut paths);
        assert_eq!(
            paths,
            vec![
                "alice".to_string(),
                "alice/shared".to_string(),
                "alice/shared/docs".to_string(),
                "alice/shared/docs/topic".to_string(),
            ]
        );
    }

    #[test]
    fn shared_projection_shows_full_subtree_once_shared_root_is_reached() {
        let d = tempdir().unwrap();
        let sym = d.path().join("sym");
        fs::create_dir_all(d.path().join("alice/shared/docs/topic")).unwrap();
        fs::create_dir_all(d.path().join("alice/shared/assets/img")).unwrap();
        fs::create_dir_all(d.path().join("alice/private/secret")).unwrap();

        let requested: TreePath = "alice".parse().unwrap();
        let shared_paths = vec!["alice/shared".parse().unwrap()];
        let node = walk_shared_projection(
            d.path(),
            &sym,
            &requested,
            &shared_paths,
            &WalkOptions {
                depth: Some(8),
                include_archived: false,
                include_branches: false,
                include_activity: false,
            },
        )
        .unwrap();

        let mut paths = Vec::new();
        collect_paths(&node, &mut paths);
        assert_eq!(
            paths,
            vec![
                "alice".to_string(),
                "alice/shared".to_string(),
                "alice/shared/assets".to_string(),
                "alice/shared/assets/img".to_string(),
                "alice/shared/docs".to_string(),
                "alice/shared/docs/topic".to_string(),
            ]
        );
        assert!(!paths.iter().any(|p| p.starts_with("alice/private")));
    }
}
