//! Platform services the UI needs from Android (file picking, secrets, starting the game…).
use launcher_core::accounts::SecretStore;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct PickedFile {
    pub kind: String,
    /// Copy of the picked file inside the app's private imports directory.
    pub path: PathBuf,
    pub display_name: String,
}

pub trait Platform: Send + Sync {
    /// Opens the system file picker (Storage Access Framework). The result arrives through `take_picked`.
    fn pick_file(&self, kind: &str);
    /// Lets the user save `src` somewhere (Storage Access Framework "create document").
    fn export_file(&self, src: &Path, name: &str, mime: &str);
    fn start_game(&self, plan: &Path) -> Result<(), String>;
    fn open_url(&self, url: &str);
    fn total_ram_mb(&self) -> u32;
    /// Usable game area in pixels (screen minus cutouts and system bars) for the controls editor aspect.
    fn usable_size(&self) -> (u32, u32);
    fn show_keyboard(&self, show: bool);
    fn secrets(&self) -> Arc<dyn SecretStore>;
    fn data_dir(&self) -> PathBuf;
}

fn queue() -> &'static Mutex<VecDeque<PickedFile>> {
    static Q: OnceLock<Mutex<VecDeque<PickedFile>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(VecDeque::new()))
}
pub fn push_picked(f: PickedFile) { queue().lock().unwrap().push_back(f); }
pub fn take_picked() -> Option<PickedFile> { queue().lock().unwrap().pop_front() }

static CTX: OnceLock<egui::Context> = OnceLock::new();
pub fn set_repaint_ctx(ctx: egui::Context) { let _ = CTX.set(ctx); }
pub fn request_repaint() { if let Some(c) = CTX.get() { c.request_repaint(); } }

/// Desktop development stub: file picking and game start are not available off-device.
pub struct DesktopPlatform { pub dir: PathBuf }
impl Platform for DesktopPlatform {
    fn pick_file(&self, _kind: &str) { launcher_core::logs::warn("File picking is only available on Android."); }
    fn export_file(&self, _s: &Path, _n: &str, _m: &str) { launcher_core::logs::warn("Export is only available on Android."); }
    fn start_game(&self, _p: &Path) -> Result<(), String> { Err("Launching the game is only possible on an Android device.".into()) }
    fn open_url(&self, _u: &str) {}
    fn total_ram_mb(&self) -> u32 { 8192 }
    fn usable_size(&self) -> (u32, u32) { (2340, 1080) }
    fn show_keyboard(&self, _s: bool) {}
    fn secrets(&self) -> Arc<dyn SecretStore> { Arc::new(launcher_core::accounts::MemorySecretStore::default()) }
    fn data_dir(&self) -> PathBuf { self.dir.clone() }
}
