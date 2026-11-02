use crate::error::{Error, Result};
use crate::paths::Dirs;
use crate::store::{load_json, save_json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GraphicsBackend {
    #[default]
    Default,
    OpenGl,
    Vulkan,
}

impl GraphicsBackend {
    /// Value for the client's `--graphicsBackend` option (read from PreferredGraphicsApi in the supplied JAR).
    pub fn cli_value(self) -> Option<&'static str> {
        match self {
            GraphicsBackend::Default => None,
            GraphicsBackend::OpenGl => Some("opengl"),
            GraphicsBackend::Vulkan => Some("vulkan"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    #[default]
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PerfPreset {
    Battery,
    #[default]
    Balanced,
    Quality,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// Minecraft version id (must match the imported client JAR).
    pub version_id: String,
    /// Runtime directory name under runtimes/ (None = best match).
    pub runtime: Option<String>,
    pub ram_mb: u32,
    pub jvm_args: String,
    pub game_args: String,
    pub width: u32,
    pub height: u32,
    pub render_scale: f32,
    pub graphics_backend: GraphicsBackend,
    pub loader: Loader,
    pub perf_preset: PerfPreset,
    pub layout_id: Option<String>,
    pub account_id: Option<String>,
}

impl Profile {
    pub fn new(name: &str, version_id: &str) -> Self {
        let id: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        let mut p = Self {
            id: if id.is_empty() { "profile".into() } else { id },
            name: name.into(), version_id: version_id.into(), runtime: None,
            ram_mb: 2048, jvm_args: String::new(), game_args: String::new(),
            width: 0, height: 0, render_scale: 1.0,
            graphics_backend: GraphicsBackend::Default, loader: Loader::Vanilla,
            perf_preset: PerfPreset::Balanced, layout_id: None, account_id: None,
        };
        p.apply_preset(PerfPreset::Balanced);
        p
    }

    pub fn apply_preset(&mut self, preset: PerfPreset) {
        self.perf_preset = preset;
        match preset {
            PerfPreset::Battery => { self.render_scale = 0.6; self.ram_mb = self.ram_mb.min(2048); }
            PerfPreset::Balanced => { self.render_scale = 0.8; }
            PerfPreset::Quality => { self.render_scale = 1.0; }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceLimits {
    pub total_ram_mb: u32,
}

impl DeviceLimits {
    /// Practical ceiling: Android needs headroom for the system, the launcher and native/GPU memory.
    pub fn max_heap_mb(&self) -> u32 { (self.total_ram_mb as u64 * 60 / 100) as u32 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub error: bool,
    pub message: String,
}

pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => { quote = Some(c); any = true; }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() { out.push(std::mem::take(&mut cur)); any = false; }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() { out.push(cur); }
    out
}

pub fn validate(p: &Profile, limits: DeviceLimits) -> Vec<Issue> {
    let mut v = Vec::new();
    let mut err = |m: String| v.push(Issue { error: true, message: m });
    if p.name.trim().is_empty() { err("Profile name is empty.".into()); }
    if p.version_id.trim().is_empty() { err("No Minecraft version selected. Import a client JAR first.".into()); }
    let max = limits.max_heap_mb();
    if p.ram_mb < 512 { err("RAM must be at least 512 MB.".into()); }
    if p.ram_mb > max {
        err(format!("RAM {} MB exceeds the safe limit for this device ({} MB of {} MB total). Lower it to avoid the system killing the game.", p.ram_mb, max, limits.total_ram_mb));
    }
    if !(0.25..=1.5).contains(&p.render_scale) { err("Render scale must be between 0.25 and 1.5.".into()); }
    if (p.width != 0 && p.width < 320) || (p.height != 0 && p.height < 240) { err("Resolution is too small (minimum 320x240, or 0 for automatic).".into()); }
    for a in split_args(&p.jvm_args) {
        if a.starts_with("-Xmx") || a.starts_with("-Xms") { err(format!("Remove '{a}' from JVM arguments: use the RAM setting instead.")); }
        else if !a.starts_with('-') { err(format!("JVM argument '{a}' must start with '-'.")); }
        else if a.starts_with("-javaagent") || a.starts_with("-agentpath") { err(format!("'{a}' is not allowed in custom JVM arguments.")); }
    }
    v
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProfileFile {
    pub selected: Option<String>,
    pub profiles: Vec<Profile>,
}

pub struct ProfileStore {
    pub file: ProfileFile,
    dirs: Dirs,
}

impl ProfileStore {
    pub fn empty(dirs: &Dirs) -> Self { Self { file: ProfileFile::default(), dirs: dirs.clone() } }
    pub fn load(dirs: &Dirs) -> Result<Self> {
        Ok(Self { file: load_json(&dirs.profiles_file())?, dirs: dirs.clone() })
    }
    pub fn save(&self) -> Result<()> { save_json(&self.dirs.profiles_file(), &self.file) }

    pub fn add(&mut self, mut p: Profile) -> Result<String> {
        let base = p.id.clone();
        let mut n = 1;
        while self.file.profiles.iter().any(|x| x.id == p.id) { n += 1; p.id = format!("{base}-{n}"); }
        std::fs::create_dir_all(self.dirs.instance(&p.id))?;
        let id = p.id.clone();
        self.file.profiles.push(p);
        if self.file.selected.is_none() { self.file.selected = Some(id.clone()); }
        self.save()?;
        Ok(id)
    }
    pub fn get(&self, id: &str) -> Option<&Profile> { self.file.profiles.iter().find(|p| p.id == id) }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Profile> { self.file.profiles.iter_mut().find(|p| p.id == id) }
    pub fn selected(&self) -> Option<&Profile> { self.file.selected.as_deref().and_then(|id| self.get(id)) }

    pub fn duplicate(&mut self, id: &str) -> Result<String> {
        let mut p = self.get(id).cloned().ok_or_else(|| Error::invalid("profile not found"))?;
        p.name = format!("{} copy", p.name);
        p.id = format!("{}-copy", p.id);
        self.add(p) // game directory is separate and starts empty
    }
    /// Removes the profile entry. The game directory is only deleted when `delete_data` is true.
    pub fn remove(&mut self, id: &str, delete_data: bool) -> Result<()> {
        self.file.profiles.retain(|p| p.id != id);
        if self.file.selected.as_deref() == Some(id) { self.file.selected = self.file.profiles.first().map(|p| p.id.clone()); }
        if delete_data {
            let d = self.dirs.instance(id);
            if d.starts_with(self.dirs.instances()) && d.exists() { std::fs::remove_dir_all(d)?; }
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits_quoted_args() {
        assert_eq!(split_args(r#"-Da=b "-Dc=d e" -X"#), vec!["-Da=b", "-Dc=d e", "-X"]);
    }
    #[test]
    fn validates_ram_and_args() {
        let mut p = Profile::new("Test", "26.3");
        p.ram_mb = 6000;
        p.jvm_args = "-Xmx1G bad -javaagent:x.jar".into();
        let issues = validate(&p, DeviceLimits { total_ram_mb: 6000 });
        assert!(issues.len() >= 4);
        p.ram_mb = 2048; p.jvm_args.clear();
        assert!(validate(&p, DeviceLimits { total_ram_mb: 6000 }).is_empty());
    }
    #[test]
    fn unique_ids() {
        let d = tempfile::tempdir().unwrap();
        let dirs = Dirs::new(d.path());
        let mut s = ProfileStore::load(&dirs).unwrap();
        let a = s.add(Profile::new("A", "1")).unwrap();
        let b = s.add(Profile::new("A", "1")).unwrap();
        assert_ne!(a, b);
    }
}
