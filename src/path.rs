use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TreePath {
    segments: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PathError {
    #[error("empty path")]
    Empty,
    #[error("invalid segment {0:?}: {1}")]
    InvalidSegment(String, &'static str),
    #[error("reserved segment {0:?} cannot be used as an exom")]
    ReservedSegment(String),
}

impl TreePath {
    /// An empty path representing the tree root itself.
    pub fn root() -> TreePath { TreePath { segments: vec![] } }

    pub fn segments(&self) -> &[String] { &self.segments }
    pub fn is_empty(&self) -> bool { self.segments.is_empty() }
    pub fn len(&self) -> usize { self.segments.len() }

    pub fn last(&self) -> Option<&str> {
        self.segments.last().map(String::as_str)
    }

    pub fn parent(&self) -> Option<TreePath> {
        if self.segments.len() <= 1 { return None; }
        Some(TreePath { segments: self.segments[..self.segments.len() - 1].to_vec() })
    }

    pub fn join(&self, segment: &str) -> Result<TreePath, PathError> {
        validate_segment(segment)?;
        let mut s = self.segments.clone();
        s.push(segment.to_string());
        Ok(TreePath { segments: s })
    }

    pub fn to_disk_path(&self, tree_root: &Path) -> PathBuf {
        let mut p = tree_root.to_path_buf();
        for seg in &self.segments { p.push(seg); }
        p
    }

    pub fn to_cli_string(&self) -> String { self.segments.join("::") }
    pub fn to_slash_string(&self) -> String { self.segments.join("/") }
}

impl std::str::FromStr for TreePath {
    type Err = PathError;
    fn from_str(s: &str) -> Result<Self, PathError> {
        if s.is_empty() { return Err(PathError::Empty); }
        let normalized = s.replace("::", "/");
        let segments: Vec<String> = normalized
            .split('/')
            .filter(|seg| !seg.is_empty())
            .map(String::from)
            .collect();
        if segments.is_empty() { return Err(PathError::Empty); }
        for seg in &segments { validate_segment(seg)?; }
        Ok(TreePath { segments })
    }
}

impl fmt::Display for TreePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_cli_string())
    }
}

/// Maximum bytes per segment, matching POSIX `NAME_MAX` on common filesystems.
pub const MAX_SEGMENT_LEN: usize = 255;

pub fn validate_segment(seg: &str) -> Result<(), PathError> {
    if seg.is_empty() {
        return Err(PathError::InvalidSegment(seg.to_string(), "empty"));
    }
    if seg.len() > MAX_SEGMENT_LEN {
        return Err(PathError::InvalidSegment(seg.to_string(), "segment exceeds 255 bytes"));
    }
    let mut chars = seg.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphanumeric() || first == '_' || first == '-') {
        return Err(PathError::InvalidSegment(seg.to_string(), "first char must be [_A-Za-z0-9-]"));
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '@') {
            return Err(PathError::InvalidSegment(seg.to_string(), "chars must be [_A-Za-z0-9.@-]"));
        }
    }
    Ok(())
}

/// Reserved only when used as an EXOM name. Allowed as a folder segment created by `init`.
pub const RESERVED_EXOM_NAMES: &[&str] = &["sessions"];

pub fn ensure_not_reserved_as_exom(name: &str) -> Result<(), PathError> {
    if RESERVED_EXOM_NAMES.contains(&name) {
        return Err(PathError::ReservedSegment(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parses_double_colon() {
        let p: TreePath = "work::ath::lynx::orsl::main".parse().unwrap();
        assert_eq!(p.segments(), &["work","ath","lynx","orsl","main"]);
        assert_eq!(p.to_cli_string(), "work::ath::lynx::orsl::main");
        assert_eq!(p.to_slash_string(), "work/ath/lynx/orsl/main");
    }

    #[test]
    fn parses_slash() {
        let p: TreePath = "work/ath/lynx".parse().unwrap();
        assert_eq!(p.segments(), &["work","ath","lynx"]);
    }

    #[test]
    fn empty_is_error() {
        assert_eq!("".parse::<TreePath>().unwrap_err(), PathError::Empty);
    }

    #[test]
    fn whitespace_in_segment_rejected() {
        let err = "work::a b".parse::<TreePath>().unwrap_err();
        assert!(matches!(err, PathError::InvalidSegment(_, _)));
    }

    #[test]
    fn sessions_is_reserved_for_exoms() {
        assert!(ensure_not_reserved_as_exom("sessions").is_err());
        assert!(ensure_not_reserved_as_exom("main").is_ok());
    }

    #[test]
    fn join_validates_segment() {
        let p = TreePath::from_str("work").unwrap();
        assert!(p.join("ath").is_ok());
        assert!(p.join("bad segment").is_err());
    }

    #[test]
    fn mixed_separator_round_trip() {
        let p: TreePath = "work::ath/lynx".parse().unwrap();
        assert_eq!(p.segments(), &["work", "ath", "lynx"]);
    }

    #[test]
    fn to_disk_path_joins_segments() {
        let p: TreePath = "work::ath".parse().unwrap();
        let root = PathBuf::from("/root/tree");
        assert_eq!(p.to_disk_path(&root), PathBuf::from("/root/tree/work/ath"));
    }

    #[test]
    fn email_segment_is_valid() {
        let p: TreePath = "alice@company.com/projects/main".parse().unwrap();
        assert_eq!(p.segments(), &["alice@company.com", "projects", "main"]);
    }

    #[test]
    fn at_sign_not_allowed_as_first_char() {
        let err = "@alice".parse::<TreePath>().unwrap_err();
        assert!(matches!(err, PathError::InvalidSegment(_, _)));
    }

    #[test]
    fn email_segment_in_join() {
        let root = TreePath::root();
        let p = root.join("alice@company.com").unwrap();
        assert_eq!(p.segments(), &["alice@company.com"]);
    }

    #[test]
    fn email_to_disk_path() {
        let p: TreePath = "alice@company.com/projects".parse().unwrap();
        let root = std::path::PathBuf::from("/root/tree");
        assert_eq!(
            p.to_disk_path(&root),
            std::path::PathBuf::from("/root/tree/alice@company.com/projects")
        );
    }
}
