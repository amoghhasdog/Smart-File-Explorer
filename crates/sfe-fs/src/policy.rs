//! Which entries the walker and watcher ignore.

use std::path::Path;

const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_REPARSE_POINT: u32 = 0x400;
/// Cloud placeholder (OneDrive etc.): reading it would trigger a download.
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// Configurable ignore rules applied to every directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRules {
    /// Directory names skipped entirely (case-insensitive).
    pub dir_names: Vec<String>,
    /// Skip entries whose name starts with a dot.
    pub skip_dotfiles: bool,
    /// Skip entries Windows marks hidden or system.
    pub skip_hidden: bool,
}

impl Default for IgnoreRules {
    fn default() -> Self {
        Self {
            dir_names: [
                "$RECYCLE.BIN",
                "System Volume Information",
                "node_modules",
                ".git",
                ".svn",
                ".hg",
                "__pycache__",
                ".venv",
                "venv",
                ".cache",
                "$WINDOWS.~BT",
                "Windows",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            skip_dotfiles: true,
            skip_hidden: true,
        }
    }
}

impl IgnoreRules {
    /// Should an entry with this name / type be skipped? `attributes` are the
    /// raw Windows file attributes when available.
    pub fn skip(&self, name: &str, is_dir: bool, attributes: Option<u32>) -> bool {
        if self.skip_dotfiles && name.starts_with('.') {
            return true;
        }
        if is_dir && self.dir_names.iter().any(|d| d.eq_ignore_ascii_case(name)) {
            return true;
        }
        if let Some(attr) = attributes {
            if self.skip_hidden && attr & (ATTR_HIDDEN | ATTR_SYSTEM) != 0 {
                return true;
            }
            if attr & ATTR_REPARSE_POINT != 0 {
                return true;
            }
            if !is_dir && attr & ATTR_RECALL_ON_DATA_ACCESS != 0 {
                return true;
            }
        }
        false
    }

    pub fn skip_path(&self, path: &Path, is_dir: bool, attributes: Option<u32>) -> bool {
        match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => self.skip(name, is_dir, attributes),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rules() {
        let r = IgnoreRules::default();
        assert!(r.skip("node_modules", true, None));
        assert!(r.skip("NODE_MODULES", true, None));
        assert!(!r.skip("node_modules", false, None), "only dirs by name");
        assert!(r.skip(".git", true, None));
        assert!(r.skip(".hidden.txt", false, None));
        assert!(!r.skip("report.pdf", false, None));
        assert!(r.skip("report.pdf", false, Some(ATTR_HIDDEN)));
        assert!(r.skip("link", true, Some(ATTR_REPARSE_POINT)));
        assert!(r.skip("cloud.docx", false, Some(ATTR_RECALL_ON_DATA_ACCESS)));
        assert!(
            !r.skip("cloud", true, Some(ATTR_RECALL_ON_DATA_ACCESS)),
            "placeholder dirs are still walked"
        );
    }

    #[test]
    fn rules_can_be_relaxed() {
        let r = IgnoreRules {
            skip_dotfiles: false,
            skip_hidden: false,
            dir_names: vec![],
        };
        assert!(!r.skip(".git", true, Some(ATTR_HIDDEN)));
    }
}
