//! Themes and UI settings (colors, opacity, scale, motion), with import/export and reset.
use crate::error::{Error, Result};
use crate::paths::Dirs;
use crate::store::{load_json, save_json};
use serde::{Deserialize, Serialize};

pub const THEME_SCHEMA: &str = "rustcraft-theme";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontChoice { #[default] Proportional, Monospace }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub schema: String,
    pub name: String,
    pub dark: bool,
    pub accent: [u8; 3],
    pub background: [u8; 3],
    pub panel: [u8; 3],
    pub text: [u8; 3],
    pub panel_opacity: f32,
    pub corner_radius: f32,
    pub font: FontChoice,
    pub font_scale: f32,
    /// File name inside backgrounds/ (png/jpg/webp/gif). None = built-in gradient.
    pub background_image: Option<String>,
}

impl Default for Theme {
    fn default() -> Self { Self::midnight() }
}

impl Theme {
    pub fn midnight() -> Self {
        Self { schema: THEME_SCHEMA.into(), name: "Midnight".into(), dark: true, accent: [96, 165, 250], background: [10, 14, 26],
               panel: [22, 28, 46], text: [230, 235, 245], panel_opacity: 0.82, corner_radius: 14.0, font: FontChoice::Proportional, font_scale: 1.0, background_image: None }
    }
    pub fn ember() -> Self {
        Self { name: "Ember".into(), accent: [251, 146, 60], background: [20, 10, 8], panel: [42, 24, 20], ..Self::midnight() }
    }
    pub fn forest() -> Self {
        Self { name: "Forest".into(), accent: [74, 222, 128], background: [8, 20, 14], panel: [18, 40, 30], ..Self::midnight() }
    }
    pub fn daylight() -> Self {
        Self { name: "Daylight".into(), dark: false, accent: [37, 99, 235], background: [238, 242, 250], panel: [255, 255, 255], text: [20, 24, 36], panel_opacity: 0.92, ..Self::midnight() }
    }
    pub fn presets() -> Vec<Theme> { vec![Self::midnight(), Self::ember(), Self::forest(), Self::daylight()] }

    pub fn clamp(&mut self) {
        self.panel_opacity = self.panel_opacity.clamp(0.2, 1.0);
        self.corner_radius = self.corner_radius.clamp(0.0, 28.0);
        self.font_scale = self.font_scale.clamp(0.8, 1.5);
        if let Some(n) = &self.background_image {
            if crate::paths::safe_file_name(n).is_none() { self.background_image = None; }
        }
        self.name = self.name.trim().chars().take(40).collect();
        if self.name.is_empty() { self.name = "Custom".into(); }
    }

    pub fn to_json(&self) -> Result<String> { Ok(serde_json::to_string_pretty(self)?) }
    pub fn from_json(s: &str) -> Result<Theme> {
        let mut t: Theme = serde_json::from_str(s).map_err(|e| Error::invalid(format!("Not a valid theme file: {e}")))?;
        if t.schema != THEME_SCHEMA { return Err(Error::invalid("This file is not a Rustcraft theme.")); }
        t.clamp();
        Ok(t)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: Theme,
    pub ui_scale: f32,
    /// 0.25 (slow) .. 2.0 (fast)
    pub anim_speed: f32,
    pub reduced_motion: bool,
    pub background_animation: bool,
    pub microsoft_client_id: String,
    pub download_concurrency: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { theme: Theme::default(), ui_scale: 1.0, anim_speed: 1.0, reduced_motion: false, background_animation: true, microsoft_client_id: String::new(), download_concurrency: 6 }
    }
}

impl AppSettings {
    pub fn clamp(&mut self) {
        self.ui_scale = self.ui_scale.clamp(0.7, 1.8);
        self.anim_speed = self.anim_speed.clamp(0.25, 2.0);
        self.download_concurrency = self.download_concurrency.clamp(1, 16);
        self.theme.clamp();
    }
    /// Effective motion multiplier: 0 when reduced motion is on (transitions become instant).
    pub fn motion(&self) -> f32 { if self.reduced_motion { 0.0 } else { self.anim_speed } }
    pub fn load(dirs: &Dirs) -> Result<Self> { let mut s: Self = load_json(&dirs.settings_file())?; s.clamp(); Ok(s) }
    pub fn save(&self, dirs: &Dirs) -> Result<()> { save_json(&dirs.settings_file(), self) }
}

pub fn save_theme(dirs: &Dirs, t: &Theme) -> Result<()> {
    let slug = crate::controls::layout_slug(&t.name);
    crate::store::write_atomic(&dirs.themes().join(format!("{slug}.json")), t.to_json()?.as_bytes())
}
pub fn list_themes(dirs: &Dirs) -> Vec<Theme> {
    let mut v: Vec<Theme> = std::fs::read_dir(dirs.themes()).map(|rd| rd.flatten()
        .filter_map(|e| std::fs::read_to_string(e.path()).ok().and_then(|s| Theme::from_json(&s).ok())).collect()).unwrap_or_default();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}
pub fn delete_theme(dirs: &Dirs, name: &str) -> Result<()> {
    let slug = crate::controls::layout_slug(name);
    Ok(std::fs::remove_file(dirs.themes().join(format!("{slug}.json")))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_roundtrip_and_clamp() {
        let mut t = Theme::ember(); t.panel_opacity = 9.0; t.background_image = Some("../x.png".into());
        let j = serde_json::to_string(&t).unwrap();
        let t2 = Theme::from_json(&j).unwrap();
        assert_eq!(t2.panel_opacity, 1.0);
        assert!(t2.background_image.is_none());
        assert!(Theme::from_json("{}").is_err());
    }
    #[test]
    fn reduced_motion_zeroes_speed() {
        let mut s = AppSettings::default();
        assert_eq!(s.motion(), 1.0);
        s.reduced_motion = true;
        assert_eq!(s.motion(), 0.0);
    }
}
