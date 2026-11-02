use std::path::{Path, PathBuf};

/// All launcher data lives under one root (the app's private files dir on Android).
#[derive(Debug, Clone)]
pub struct Dirs {
    pub root: PathBuf,
}

impl Dirs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn versions(&self) -> PathBuf { self.root.join("versions") }
    pub fn version_dir(&self, id: &str) -> PathBuf { self.versions().join(id) }
    pub fn libraries(&self) -> PathBuf { self.root.join("libraries") }
    pub fn assets(&self) -> PathBuf { self.root.join("assets") }
    pub fn runtimes(&self) -> PathBuf { self.root.join("runtimes") }
    pub fn lwjgl_override(&self) -> PathBuf { self.root.join("lwjgl") }
    pub fn instances(&self) -> PathBuf { self.root.join("instances") }
    pub fn instance(&self, profile_id: &str) -> PathBuf { self.instances().join(profile_id) }
    pub fn profiles_file(&self) -> PathBuf { self.root.join("profiles.json") }
    pub fn accounts_file(&self) -> PathBuf { self.root.join("accounts.json") }
    pub fn servers_file(&self) -> PathBuf { self.root.join("servers.json") }
    pub fn settings_file(&self) -> PathBuf { self.root.join("settings.json") }
    pub fn themes(&self) -> PathBuf { self.root.join("themes") }
    pub fn layouts(&self) -> PathBuf { self.root.join("layouts") }
    pub fn backgrounds(&self) -> PathBuf { self.root.join("backgrounds") }
    pub fn logs(&self) -> PathBuf { self.root.join("logs") }
    pub fn imports(&self) -> PathBuf { self.root.join("imports") }
    pub fn launch_plan(&self) -> PathBuf { self.root.join("run").join("launch_plan.json") }

    pub fn ensure_all(&self) -> std::io::Result<()> {
        for d in [
            self.versions(), self.libraries(), self.assets(), self.runtimes(),
            self.lwjgl_override(), self.instances(), self.themes(), self.layouts(),
            self.backgrounds(), self.logs(), self.imports(), self.root.join("run"),
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

/// Reject names that could escape a directory when used as a file name.
pub fn safe_file_name(name: &str) -> Option<&str> {
    let p = Path::new(name);
    let base = p.file_name()?.to_str()?;
    if base != name || base.is_empty() || base == "." || base == ".." || base.contains('\0') {
        return None;
    }
    Some(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal() {
        assert!(safe_file_name("../x").is_none());
        assert!(safe_file_name("a/b").is_none());
        assert!(safe_file_name("..").is_none());
        assert_eq!(safe_file_name("mod.jar"), Some("mod.jar"));
    }
}
