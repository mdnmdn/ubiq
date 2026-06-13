//! Project discovery: locate the project root and load its unified config.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::UnifiedConfig;

/// Default name of the unified config file at the project root.
pub const CONFIG_FILE_NAME: &str = ".agent-manager.toml";

/// A discovered project: its root directory and parsed config.
#[derive(Debug, Clone)]
pub struct Project {
    /// Absolute path to the project root.
    pub root: PathBuf,
    /// Parsed unified config.
    pub config: UnifiedConfig,
}

impl Project {
    /// Walk upwards from `start` looking for a [`CONFIG_FILE_NAME`].
    pub fn discover(start: &Path) -> Result<Option<Project>> {
        let mut current = Some(start.to_path_buf());
        while let Some(dir) = current {
            let candidate = dir.join(CONFIG_FILE_NAME);
            if candidate.is_file() {
                return Ok(Some(Project::load(&dir)?));
            }
            current = dir.parent().map(|p| p.to_path_buf());
        }
        Ok(None)
    }

    /// Load a project from an explicit root directory.
    pub fn load(root: &Path) -> Result<Project> {
        let path = root.join(CONFIG_FILE_NAME);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let config: UnifiedConfig =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Project {
            root: root.to_path_buf(),
            config,
        })
    }
}
