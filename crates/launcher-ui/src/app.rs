//! Application state, background tasks, theme application, shell (nav, transitions, toasts).
use crate::platform::{take_picked, PickedFile, Platform};
use egui::{vec2, Align2, Color32, Context, CornerRadius, FontFamily, FontId, Margin, Rect, Response, Sense, TextStyle, Ui};
use launcher_core::accounts::*;
use launcher_core::controls::{self, Layout};
use launcher_core::download::Progress;
use launcher_core::jar_inspect::{self, JarReport};
use launcher_core::launch::{build_plan, LaunchAccount, LaunchInputs};
use launcher_core::logs;
use launcher_core::mods::ModInfo;
use launcher_core::prepare;
use launcher_core::profiles::*;
use launcher_core::runtime::{self, RuntimeInfo};
use launcher_core::servers::{self, ServerFile, ServerStatus};
use launcher_core::theme::*;
use launcher_core::Dirs;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page { Play, Profiles, Accounts, Mods, ResourcePacks, Shaders, Controls, Settings, Logs }
impl Page {
    pub const ALL: [Page; 9] = [Page::Play, Page::Profiles, Page::Accounts, Page::Mods, Page::ResourcePacks, Page::Shaders, Page::Controls, Page::Settings, Page::Logs];
    pub fn label(self) -> &'static str {
        match self { Page::Play => "Play", Page::Profiles => "Profiles", Page::Accounts => "Accounts", Page::Mods => "Mods", Page::ResourcePacks => "Resource Packs", Page::Shaders => "Shaders", Page::Controls => "Controls", Page::Settings => "Settings", Page::Logs => "Logs" }
    }
}

/// Visual constants derived from the theme; Copy so pages can use them while borrowing `self`.
#[derive(Clone, Copy)]
pub struct Fx { pub accent: Color32, pub panel: Color32, pub text: Color32, pub radius: u8, pub motion: f32 }
impl Fx {
    pub fn dur(&self, base: f32) -> f32 { if self.motion <= 0.0 { 0.0 } else { base / self.motion } }
}

pub struct LaunchReady { pub plan_path: PathBuf, pub cmd: String }

pub enum Msg {
    Launch(Result<LaunchReady, String>),
    DeviceCode(DeviceCode),
    MsDone(Result<(String, String), String>),
    Status(String, Result<ServerStatus, String>),
    Info(String),
    Err(String),
}

pub struct Toast { pub text: String, pub error: bool, pub until: f64 }

#[derive(Default)]
pub struct MsState { pub dc: Option<DeviceCode>, pub cancel: Arc<AtomicBool>, pub working: bool }

#[derive(Default)]
pub struct Forms {
    pub new_profile: String, pub offline_name: String, pub server_name: String, pub server_addr: String,
    pub direct_addr: String, pub theme_name: String, pub layout_name: String, pub runtime_name: String,
    pub custom_key: String,
}

pub struct App {
    pub ctx: Context,
    pub platform: Arc<dyn Platform>,
    pub dirs: Dirs,
    pub rt: Arc<tokio::runtime::Runtime>,
    pub tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub settings: AppSettings,
    pub settings_dirty: bool,
    pub theme_dirty: bool,
    pub fx: Fx,
    pub profiles: ProfileStore,
    pub accounts: AccountStore,
    pub servers: ServerFile,
    pub server_status: HashMap<String, Result<ServerStatus, String>>,
    pub page: Page,
    pub page_changed_at: f64,
    pub jar_report: Option<JarReport>,
    pub runtimes: Vec<RuntimeInfo>,
    pub progress: Option<Arc<Progress>>,
    pub busy: Option<String>,
    pub toasts: Vec<Toast>,
    pub ms: MsState,
    pub forms: Forms,
    pub mods: Vec<ModInfo>,
    pub rp_files: Vec<String>,
    pub rp_enabled: Vec<String>,
    pub shader_files: Vec<String>,
    pub saved_layouts: Vec<String>,
    pub saved_themes: Vec<Theme>,
    pub layout: Layout,
    pub layout_slug: String,
    pub sel: Option<usize>,
    pub snap_on: bool,
    pub guides: Vec<controls::Guide>,
    pub game_log_tail: String,
}

fn rgb(a: [u8; 3]) -> Color32 { Color32::from_rgb(a[0], a[1], a[2]) }
pub fn contrast(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l > 140.0 { Color32::BLACK } else { Color32::WHITE }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, platform: Arc<dyn Platform>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        crate::platform::set_repaint_ctx(cc.egui_ctx.clone());
        let dirs = Dirs::new(platform.data_dir());
        let _ = dirs.ensure_all();
        let settings = AppSettings::load(&dirs).unwrap_or_default();
        let profiles = ProfileStore::load(&dirs).unwrap_or_else(|e| { logs::error(&format!("profiles.json unreadable ({e}); starting empty")); ProfileStore::empty(&dirs) });
        let accounts = AccountStore::load(&dirs).unwrap_or_else(|e| { logs::error(&format!("accounts.json unreadable ({e}); starting empty")); AccountStore::empty(&dirs) });
        let servers = servers::load(&dirs).unwrap_or_default();
        let (tx, rx) = channel();
        let rt = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(3).enable_all().build().expect("tokio runtime"));
        let jar = dirs.imports().join("client.jar");
        let jar_report = if jar.is_file() { jar_inspect::inspect(&jar).ok() } else { None };
        let fx = Self::make_fx(&settings);
        let mut app = Self {
            ctx: cc.egui_ctx.clone(), platform, runtimes: runtime::scan(&dirs), dirs, rt, tx, rx, settings, settings_dirty: false, theme_dirty: true, fx,
            profiles, accounts, servers, server_status: HashMap::new(), page: Page::Play, page_changed_at: -10.0, jar_report,
            progress: None, busy: None, toasts: vec![], ms: MsState::default(), forms: Forms::default(), mods: vec![], rp_files: vec![], rp_enabled: vec![],
            shader_files: vec![], saved_layouts: vec![], saved_themes: vec![], layout: controls::default_layout(), layout_slug: "default".into(), sel: None,
            snap_on: true, guides: vec![], game_log_tail: String::new(),
        };
        app.refresh_all();
        logs::info("Launcher started");
        app
    }

    fn make_fx(s: &AppSettings) -> Fx {
        Fx { accent: rgb(s.theme.accent), panel: rgb(s.theme.panel).gamma_multiply(s.theme.panel_opacity), text: rgb(s.theme.text), radius: s.theme.corner_radius as u8, motion: s.motion() }
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error { logs::error(&text) } else { logs::info(&text) }
        let now = self.ctx.input(|i| i.time);
        self.toasts.push(Toast { text, error, until: now + if error { 8.0 } else { 4.0 } });
    }

    pub fn refresh_all(&mut self) {
        self.runtimes = runtime::scan(&self.dirs);
        self.saved_layouts = controls::list_layouts(&self.dirs);
        self.saved_themes = list_themes(&self.dirs);
        if let Some(p) = self.profiles.selected().cloned() {
            let inst = self.dirs.instance(&p.id);
            self.mods = launcher_core::mods::scan(&inst.join("mods"));
            self.rp_files = launcher_core::packs::list_files(&inst.join("resourcepacks"));
            self.rp_enabled = launcher_core::packs::enabled_resource_packs(&inst);
            self.shader_files = launcher_core::packs::list_files(&inst.join("shaderpacks"));
            if let Some(id) = &p.layout_id { if let Ok(l) = controls::load_layout(&self.dirs, id) { if self.layout_slug != *id { self.layout = l; self.layout_slug = id.clone(); } } }
        } else { self.mods.clear(); self.rp_files.clear(); self.shader_files.clear(); }
    }

    pub fn dur(&self, base: f32) -> f32 { self.fx.dur(base) }

    // ------------------------------------------------------------ theme
    fn apply_theme(&mut self, ctx: &Context) {
        let s = &self.settings;
        let t = &s.theme;
        let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        let (accent, panel, text) = (rgb(t.accent), rgb(t.panel), rgb(t.text));
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = panel;
        v.extreme_bg_color = rgb(t.background);
        v.override_text_color = Some(text);
        v.selection.bg_fill = accent.gamma_multiply(0.55);
        v.hyperlink_color = accent;
        let r = CornerRadius::same(t.corner_radius as u8);
        v.window_corner_radius = r;
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] { w.corner_radius = r; }
        v.widgets.inactive.bg_fill = panel.gamma_multiply(1.25);
        v.widgets.inactive.weak_bg_fill = panel.gamma_multiply(1.25);
        v.widgets.hovered.bg_fill = accent.gamma_multiply(0.45);
        v.widgets.hovered.weak_bg_fill = accent.gamma_multiply(0.45);
        v.widgets.active.bg_fill = accent;
        v.widgets.active.weak_bg_fill = accent;
        ctx.set_visuals(v);
        let family = if t.font == FontChoice::Monospace { FontFamily::Monospace } else { FontFamily::Proportional };
        let sc = t.font_scale;
        ctx.style_mut(|st| {
            for (ts, size) in [(TextStyle::Heading, 26.0), (TextStyle::Body, 16.0), (TextStyle::Button, 16.0), (TextStyle::Small, 13.0), (TextStyle::Monospace, 15.0)] {
                let fam = if ts == TextStyle::Monospace { FontFamily::Monospace } else { family.clone() };
                st.text_styles.insert(ts, FontId::new(size * sc, fam));
            }
            st.spacing.interact_size.y = 38.0; // touch-sized targets
            st.spacing.button_padding = vec2(14.0, 8.0);
            st.spacing.item_spacing = vec2(10.0, 10.0);
            st.spacing.slider_width = 200.0;
            st.scroll_animation = egui::style::ScrollAnimation::none();
        });
        ctx.set_zoom_factor(s.ui_scale);
        self.fx = Self::make_fx(&self.settings);
        self.theme_dirty = false;
    }

    fn paint_background(&mut self, ctx: &Context) {
        let screen = ctx.screen_rect();
        let t = &self.settings.theme;
        let (bg, acc) = (rgb(t.background), rgb(t.accent));
        let time = ctx.input(|i| i.time) as f32;
        let painter = ctx.layer_painter(egui::LayerId::background());
        let top = bg;
        let bottom = bg.lerp_to_gamma(acc, 0.18);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(screen.left_top(), top);
        mesh.colored_vertex(screen.right_top(), top);
        mesh.colored_vertex(screen.right_bottom(), bottom);
        mesh.colored_vertex(screen.left_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
        if let Some(name) = &t.background_image {
            let path = self.dirs.backgrounds().join(name);
            if path.is_file() {
                egui::Area::new(egui::Id::new("bg_image")).order(egui::Order::Background).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
                    egui::Image::new(format!("file://{}", path.display())).fit_to_exact_size(screen.size()).paint_at(ui, screen);
                });
            }
        }
        if self.settings.background_animation && self.settings.motion() > 0.0 {
            let sp = self.settings.motion();
            for i in 0..12 {
                let f = i as f32;
                let x = ((time * 0.04 * sp + f * 1.7).sin() * 0.5 + 0.5) * screen.width();
                let y = ((time * 0.03 * sp + f * 2.3).cos() * 0.5 + 0.5) * screen.height();
                painter.circle_filled(egui::pos2(x, y), 40.0 + (i % 5) as f32 * 22.0, acc.gamma_multiply(0.05));
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(40)); // ~25 fps, only while enabled
        }
    }

    // ------------------------------------------------------------ messages & picked files
    fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Launch(Ok(r)) => {
                    self.busy = None; self.progress = None;
                    logs::info(&format!("Launching: {}", r.cmd));
                    if let Err(e) = self.platform.start_game(&r.plan_path) { self.toast(format!("Could not start the game: {e}"), true); }
                }
                Msg::Launch(Err(e)) => { self.busy = None; self.progress = None; self.toast(e, true); }
                Msg::DeviceCode(dc) => self.ms.dc = Some(dc),
                Msg::MsDone(r) => {
                    self.ms = MsState::default();
                    match r {
                        Ok((uuid, name)) => match self.accounts.upsert_microsoft(&uuid, &name) { Ok(_) => self.toast(format!("Signed in as {name}"), false), Err(e) => self.toast(e.to_string(), true) },
                        Err(e) => self.toast(e, true),
                    }
                }
                Msg::Status(addr, r) => { self.server_status.insert(addr, r); }
                Msg::Info(s) => { self.toast(s, false); self.refresh_all(); }
                Msg::Err(s) => self.toast(s, true),
            }
        }
        while let Some(f) = take_picked() { self.on_picked(f); }
    }

    fn selected_instance(&self) -> Option<PathBuf> { self.profiles.selected().map(|p| self.dirs.instance(&p.id)) }

    fn on_picked(&mut self, f: PickedFile) {
        let res: Result<String, String> = (|| -> Result<String, String> {
            let e = |x: launcher_core::Error| x.to_string();
            match f.kind.as_str() {
                "clientjar" => {
                    let dest = self.dirs.imports().join("client.jar");
                    std::fs::copy(&f.path, &dest).map_err(|x| x.to_string())?;
                    let rep = jar_inspect::inspect(&dest).map_err(e)?;
                    let id = rep.version.id.clone();
                    if self.profiles.file.profiles.is_empty() {
                        self.profiles.add(Profile::new(&format!("Minecraft {id}"), &id)).map_err(e)?;
                    }
                    self.jar_report = Some(rep);
                    Ok(format!("Client JAR imported: Minecraft {id}"))
                }
                "runtime" => {
                    let (dirs, tx, path) = (self.dirs.clone(), self.tx.clone(), f.path.clone());
                    let name = std::path::Path::new(&f.display_name).file_stem().map(|s| s.to_string_lossy().replace(".tar", "")).unwrap_or_else(|| "runtime".into());
                    self.rt.spawn_blocking(move || {
                        let _ = tx.send(match runtime::import_archive(&dirs, &path, &name) {
                            Ok(r) => Msg::Info(format!("Runtime '{}' installed (Java {})", r.name, r.java_major.map(|m| m.to_string()).unwrap_or("?".into()))),
                            Err(e) => Msg::Err(e.to_string()),
                        });
                        crate::platform::request_repaint();
                    });
                    Ok("Importing runtime…".into())
                }
                "lwjgl" => {
                    let n = launcher_core::paths::safe_file_name(&f.display_name).ok_or("Invalid file name")?;
                    if !(n.ends_with(".jar") || n.ends_with(".so")) { return Err("LWJGL files must be .jar or .so".into()); }
                    std::fs::copy(&f.path, self.dirs.lwjgl_override().join(n)).map_err(|x| x.to_string())?;
                    Ok(format!("Added {n} to the Android LWJGL set"))
                }
                k @ ("mod" | "resourcepack" | "shaderpack") => {
                    let inst = self.selected_instance().ok_or("Create a profile first")?;
                    match k {
                        "mod" => { launcher_core::mods::import(&inst.join("mods"), &f.path, &f.display_name).map_err(e)?; }
                        "resourcepack" => { launcher_core::packs::import_resource_pack(&inst, &f.path, &f.display_name).map_err(e)?; }
                        _ => { launcher_core::packs::import_shader_pack(&inst, &f.path, &f.display_name).map_err(e)?; }
                    }
                    Ok(format!("Imported {}", f.display_name))
                }
                "background" => {
                    let n = launcher_core::paths::safe_file_name(&f.display_name).ok_or("Invalid file name")?.to_string();
                    std::fs::copy(&f.path, self.dirs.backgrounds().join(&n)).map_err(|x| x.to_string())?;
                    self.settings.theme.background_image = Some(n);
                    self.settings_dirty = true; self.theme_dirty = true;
                    Ok("Background set".into())
                }
                "theme" => {
                    let t = Theme::from_json(&std::fs::read_to_string(&f.path).map_err(|x| x.to_string())?).map_err(e)?;
                    self.settings.theme = t; self.settings_dirty = true; self.theme_dirty = true;
                    Ok("Theme imported".into())
                }
                "layout" => {
                    let l = Layout::from_json(&std::fs::read_to_string(&f.path).map_err(|x| x.to_string())?).map_err(e)?;
                    self.layout_slug = controls::save_layout(&self.dirs, &l).map_err(e)?;
                    self.layout = l; self.sel = None;
                    Ok("Layout imported".into())
                }
                other => Err(format!("Unknown import type '{other}'")),
            }
        })();
        match res { Ok(m) => self.toast(m, false), Err(m) => self.toast(m, true) }
        let _ = std::fs::remove_file(&f.path);
        self.refresh_all();
    }

    // ------------------------------------------------------------ launching
    pub fn play(&mut self, quick: Option<(String, u16)>) {
        if self.busy.is_some() { return; }
        let Some(profile) = self.profiles.selected().cloned() else { return self.toast("Create a profile first (Profiles page).", true) };
        let account = profile.account_id.as_ref().and_then(|id| self.accounts.file.accounts.iter().find(|a| &a.id == id)).or_else(|| self.accounts.selected()).cloned();
        let Some(account) = account else { return self.toast("Add an account first (Accounts page).", true) };
        let jar = self.dirs.imports().join("client.jar");
        if !jar.is_file() { return self.toast("Import your client JAR first (Play page).", true); }
        let limits = DeviceLimits { total_ram_mb: self.platform.total_ram_mb() };
        if let Some(i) = validate(&profile, limits).into_iter().find(|i| i.error) { return self.toast(i.message, true); }
        let progress = Progress::new();
        self.progress = Some(progress.clone());
        self.busy = Some("Preparing".into());
        let (dirs, tx, secrets, cid) = (self.dirs.clone(), self.tx.clone(), self.platform.secrets(), self.settings.microsoft_client_id.clone());
        let screen = self.platform.usable_size();
        self.rt.spawn(async move {
            let r = launch_task(dirs, jar, profile, account, secrets, cid, screen, quick, progress).await;
            let _ = tx.send(Msg::Launch(r));
            crate::platform::request_repaint();
        });
    }

    pub fn start_microsoft_login(&mut self) {
        let cid = self.settings.microsoft_client_id.clone();
        let auth = match MicrosoftAuth::new(&cid) { Ok(a) => a, Err(e) => return self.toast(e.to_string(), true) };
        self.ms = MsState { dc: None, cancel: Arc::new(AtomicBool::new(false)), working: true };
        let (tx, cancel, secrets) = (self.tx.clone(), self.ms.cancel.clone(), self.platform.secrets());
        self.rt.spawn(async move {
            let res: Result<(String, String), String> = async {
                let dc = auth.start_device_code().await.map_err(|e| e.to_string())?;
                let _ = tx.send(Msg::DeviceCode(dc.clone()));
                crate::platform::request_repaint();
                let (at, rt) = auth.poll_device_code(&dc, &|| cancel.load(Ordering::Relaxed)).await.map_err(|e| e.to_string())?;
                let s = auth.minecraft_login(&at, &rt).await.map_err(|e| e.to_string())?;
                store_ms_session(&*secrets, &s).map_err(|e| e.to_string())?;
                Ok((s.uuid, s.username))
            }.await;
            let _ = tx.send(Msg::MsDone(res));
            crate::platform::request_repaint();
        });
    }

    pub fn ping_server(&mut self, host: String, port: u16) {
        let tx = self.tx.clone();
        let protocol = self.jar_report.as_ref().and_then(|r| r.version.protocol_version).unwrap_or(0);
        self.rt.spawn(async move {
            let r = servers::ping(&host, port, protocol).await.map_err(|e| e.to_string());
            let _ = tx.send(Msg::Status(format!("{host}:{port}"), r));
            crate::platform::request_repaint();
        });
    }

    // ------------------------------------------------------------ frame
    pub fn set_page(&mut self, p: Page) {
        if p != self.page {
            self.page = p;
            self.page_changed_at = self.ctx.input(|i| i.time);
            self.refresh_all();
        }
    }
}

fn now_unix() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

pub fn store_ms_session(secrets: &dyn SecretStore, s: &McSession) -> launcher_core::Result<()> {
    let id = format!("ms-{}", s.uuid.replace('-', ""));
    secrets.set(&format!("{id}.refresh"), &s.ms_refresh_token)?;
    secrets.set(&format!("{id}.mc"), &serde_json::json!({"t": s.mc_access_token, "exp": now_unix() + s.mc_expires_in}).to_string())
}

async fn minecraft_token(secrets: &dyn SecretStore, cid: &str, account: &Account) -> Result<String, String> {
    let key = format!("{}.mc", account.id);
    if let Some(v) = secrets.get(&key).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
        if let (Some(t), Some(exp)) = (v["t"].as_str(), v["exp"].as_u64()) {
            if exp > now_unix() + 120 { logs::register_secret(t); return Ok(t.to_string()); }
        }
    }
    let refresh = secrets.get(&format!("{}.refresh", account.id)).ok_or("This Microsoft account needs to sign in again (Accounts page).")?;
    let auth = MicrosoftAuth::new(cid).map_err(|e| e.to_string())?;
    let (at, rt) = auth.refresh(&refresh).await.map_err(|e| e.to_string())?;
    let s = auth.minecraft_login(&at, &rt).await.map_err(|e| e.to_string())?;
    store_ms_session(secrets, &s).map_err(|e| e.to_string())?;
    Ok(s.mc_access_token)
}

#[allow(clippy::too_many_arguments)]
async fn launch_task(dirs: Dirs, jar: PathBuf, profile: Profile, account: Account, secrets: Arc<dyn SecretStore>, cid: String,
                     screen: (u32, u32), quick: Option<(String, u16)>, p: Arc<Progress>) -> Result<LaunchReady, String> {
    let prepared = prepare::prepare(&dirs, &jar, p.clone()).await.map_err(|e| e.to_string())?;
    if prepared.report.version.id != profile.version_id {
        return Err(format!("Profile '{}' is set to {} but the imported JAR is {}. Update the profile's version.", profile.name, profile.version_id, prepared.report.version.id));
    }
    for w in &prepared.warnings { logs::warn(w); }
    let required = prepared.manifest.java_version.as_ref().map(|j| j.major_version).unwrap_or(0).max(prepared.report.required_java);
    let runtime = runtime::select(&dirs, profile.runtime.as_deref(), required).map_err(|e| e.to_string())?;
    p.set_label("Preparing launch");
    let la = match account.kind {
        AccountKind::Offline => LaunchAccount { username: account.username.clone(), uuid: account.uuid.clone(), access_token: "0".into(), microsoft: false },
        AccountKind::Microsoft => {
            p.set_label("Signing in");
            let token = minecraft_token(&*secrets, &cid, &account).await?;
            LaunchAccount { username: account.username.clone(), uuid: account.uuid.clone(), access_token: token, microsoft: true }
        }
    };
    let plan = build_plan(&LaunchInputs { dirs: &dirs, profile: &profile, manifest: &prepared.manifest, account: &la, runtime: &runtime, screen, quick_play: quick }).map_err(|e| e.to_string())?;
    let plan_path = plan.write(&dirs).map_err(|e| e.to_string())?;
    Ok(LaunchReady { plan_path, cmd: plan.redacted_command() })
}

// ---------------------------------------------------------------------- shared widgets

pub fn fx_button(ui: &mut Ui, fx: Fx, text: &str, primary: bool) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let col = if primary { contrast(fx.accent) } else { fx.text };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, col);
    let size = galley.size() + vec2(32.0, 18.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(size.x.max(88.0), size.y.max(40.0)), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), fx.dur(0.12));
    let press = ui.ctx().animate_bool_with_time(resp.id.with("p"), resp.is_pointer_button_down_on(), fx.dur(0.08));
    let r = rect.shrink(press * 3.0);
    let base = if primary { fx.accent } else { fx.panel.lerp_to_gamma(Color32::WHITE, 0.08) };
    let fill = base.lerp_to_gamma(fx.accent, hover * if primary { 0.0 } else { 0.4 }).gamma_multiply(1.0 + hover * if primary { 0.1 } else { 0.0 });
    ui.painter().rect_filled(r, CornerRadius::same(fx.radius), fill);
    ui.painter().galley(r.center() - galley.size() / 2.0, galley, col);
    resp
}

pub fn panel<R>(ui: &mut Ui, fx: Fx, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new().fill(fx.panel).corner_radius(CornerRadius::same(fx.radius)).inner_margin(Margin::same(14)).show(ui, add).inner
}

/// Single-line text field that raises the soft keyboard when focused.
pub fn kb_edit(ui: &mut Ui, pf: &Arc<dyn Platform>, text: &mut String, hint: &str, width: f32) -> Response {
    let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width));
    if r.gained_focus() { pf.show_keyboard(true); }
    if r.lost_focus() { pf.show_keyboard(false); }
    r
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.pump();
        if self.theme_dirty { self.apply_theme(ctx); }
        self.paint_background(ctx);
        let now = ctx.input(|i| i.time);

        let nav_frame = egui::Frame::new().fill(self.fx.panel).inner_margin(Margin::same(10));
        egui::SidePanel::left("nav").exact_width(168.0).resizable(false).frame(nav_frame).show(ctx, |ui| {
            ui.add_space(6.0);
            ui.heading("Rustcraft");
            ui.add_space(8.0);
            let fx = self.fx;
            for p in Page::ALL {
                let sel = self.page == p;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                let a = ctx.animate_bool_with_time(resp.id.with("n"), sel || resp.hovered(), fx.dur(0.15));
                let bar = ctx.animate_bool_with_time(resp.id.with("b"), sel, fx.dur(0.2));
                ui.painter().rect_filled(rect, CornerRadius::same(fx.radius), fx.accent.gamma_multiply(0.10 + 0.22 * a));
                ui.painter().rect_filled(Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(4.0, 24.0 * bar)), CornerRadius::same(2), fx.accent);
                ui.painter().text(rect.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, p.label(), TextStyle::Button.resolve(ui.style()), fx.text);
                if resp.clicked() { self.set_page(p); }
            }
        });

        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(14))).show(ctx, |ui| {
            let dur = self.dur(0.28) as f64;
            let t = if dur <= 0.0 { 1.0 } else { ((now - self.page_changed_at) / dur).clamp(0.0, 1.0) };
            let e = 1.0 - (1.0 - t as f32).powi(3);
            if t < 1.0 { ctx.request_repaint(); }
            ui.set_opacity(e);
            ui.add_space((1.0 - e) * 22.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.page {
                Page::Play => self.page_play(ui),
                Page::Profiles => self.page_profiles(ui),
                Page::Accounts => self.page_accounts(ui),
                Page::Mods => self.page_mods(ui),
                Page::ResourcePacks => self.page_resource_packs(ui),
                Page::Shaders => self.page_shaders(ui),
                Page::Controls => self.page_controls(ui),
                Page::Settings => self.page_settings(ui),
                Page::Logs => self.page_logs(ui),
            });
        });

        self.toasts.retain(|t| t.until > now);
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            egui::Area::new(egui::Id::new("toasts")).anchor(Align2::RIGHT_BOTTOM, vec2(-14.0, -14.0)).order(egui::Order::Tooltip).show(ctx, |ui| {
                for t in &self.toasts {
                    let c = if t.error { Color32::from_rgb(180, 50, 50) } else { self.fx.accent.gamma_multiply(0.85) };
                    egui::Frame::new().fill(c).corner_radius(CornerRadius::same(self.fx.radius)).inner_margin(Margin::same(10)).show(ui, |ui| {
                        ui.set_max_width(380.0);
                        ui.colored_label(contrast(c), &t.text);
                    });
                }
            });
        }
        if self.busy.is_some() { ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
        if self.settings_dirty {
            self.settings.clamp();
            if let Err(e) = self.settings.save(&self.dirs) { logs::error(&format!("saving settings: {e}")); }
            self.settings_dirty = false;
            self.fx = Self::make_fx(&self.settings);
        }
    }
}
