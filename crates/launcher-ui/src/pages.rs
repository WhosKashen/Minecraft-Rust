//! The nine pages. Every control here is wired to launcher-core.
use crate::app::*;
use egui::{pos2, vec2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui};
use launcher_core::controls::{self, Binding, Element, ElementKind, MouseButton, Mode};
use launcher_core::logs;
use launcher_core::packs;
use launcher_core::profiles::*;
use launcher_core::servers::{self, ServerEntry};
use launcher_core::theme::*;

fn combo<T: PartialEq + Clone>(ui: &mut Ui, label: &str, cur: &mut T, opts: &[(T, &str)]) {
    let text = opts.iter().find(|(v, _)| v == cur).map(|(_, n)| *n).unwrap_or("?");
    egui::ComboBox::from_label(label).selected_text(text).show_ui(ui, |ui| {
        for (v, n) in opts { ui.selectable_value(cur, v.clone(), *n); }
    });
}

const KEYS: &[&str] = &["W", "A", "S", "D", "SPACE", "LSHIFT", "LCTRL", "E", "Q", "F", "T", "R", "G", "C", "V", "X", "Z", "B", "H", "M", "N", "O", "P", "SLASH", "TAB", "ENTER", "ESCAPE",
    "UP", "DOWN", "LEFT", "RIGHT", "F1", "F2", "F3", "F4", "F5", "F11", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

fn binding_kind(b: &Binding) -> u8 {
    match b { Binding::Key(_) => 0, Binding::Mouse(MouseButton::Left) => 1, Binding::Mouse(MouseButton::Right) => 2, Binding::Mouse(MouseButton::Middle) => 3, Binding::ScrollUp => 4, Binding::ScrollDown => 5 }
}

impl App {
    // ------------------------------------------------------------------ Play
    pub(crate) fn page_play(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Play");
        panel(ui, fx, |ui| {
            ui.strong("Client JAR");
            match &self.jar_report {
                Some(r) => {
                    ui.label(format!("Minecraft {} · Java {} required · protocol {}", r.version.id, r.required_java, r.version.protocol_version.map(|p| p.to_string()).unwrap_or("?".into())));
                    let mut be = vec![];
                    if r.has_vulkan_backend { be.push("Vulkan"); }
                    if r.has_opengl_backend { be.push("OpenGL"); }
                    ui.label(format!("Graphics backends in this JAR: {} · windowing: {}", be.join(" + "), if r.uses_sdl { "SDL" } else if r.uses_glfw { "GLFW" } else { "unknown" }));
                    for w in &r.warnings { ui.small(format!("⚠ {w}")); }
                    ui.collapsing("Still required on this device", |ui| { for m in &r.missing { ui.label(format!("• {m}")); } });
                }
                None => { ui.label("No client JAR imported yet."); }
            }
            if fx_button(ui, fx, if self.jar_report.is_some() { "Replace client JAR" } else { "Import client JAR" }, false).clicked() { pf.pick_file("clientjar"); }
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            let mut psel = self.profiles.file.selected.clone();
            egui::ComboBox::from_label("Profile").selected_text(self.profiles.selected().map(|p| p.name.clone()).unwrap_or("none".into())).show_ui(ui, |ui| {
                for p in &self.profiles.file.profiles { ui.selectable_value(&mut psel, Some(p.id.clone()), &p.name); }
            });
            if psel != self.profiles.file.selected { self.profiles.file.selected = psel; let _ = self.profiles.save(); self.refresh_all(); }
            let cur_acc = self.profiles.selected().and_then(|p| p.account_id.clone()).or_else(|| self.accounts.file.selected.clone());
            let mut asel = cur_acc.clone();
            egui::ComboBox::from_label("Account").selected_text(self.accounts.file.accounts.iter().find(|a| Some(&a.id) == cur_acc.as_ref()).map(|a| format!("{} ({})", a.username, if a.kind == launcher_core::accounts::AccountKind::Offline { "offline" } else { "Microsoft" })).unwrap_or("none".into())).show_ui(ui, |ui| {
                for a in &self.accounts.file.accounts { ui.selectable_value(&mut asel, Some(a.id.clone()), format!("{} ({})", a.username, if a.kind == launcher_core::accounts::AccountKind::Offline { "offline" } else { "Microsoft" })); }
            });
            if asel != cur_acc {
                self.accounts.file.selected = asel.clone(); let _ = self.accounts.save();
                if let Some(id) = self.profiles.file.selected.clone() { if let Some(p) = self.profiles.get_mut(&id) { p.account_id = asel; } let _ = self.profiles.save(); }
            }
            ui.add_space(6.0);
            if let Some(p) = self.progress.clone() {
                ui.add(egui::ProgressBar::new(p.fraction()).animate(fx.motion > 0.0).show_percentage());
                ui.label(p.label());
                if fx_button(ui, fx, "Cancel", false).clicked() { p.cancel(); }
            } else if fx_button(ui, fx, "PLAY", true).clicked() { self.play(None); }
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong("Servers");
            ui.small("Offline profiles can only join servers that accept offline (non-premium) players. Servers with online-mode enabled will refuse them; the launcher never bypasses that. Login plugins (/login, /register) work through normal chat.");
            let (mut join, mut ping, mut remove) = (None::<usize>, None::<usize>, None::<usize>);
            for (i, s) in self.servers.servers.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} — {}:{}", s.name, s.host, s.port));
                    match self.server_status.get(&format!("{}:{}", s.host, s.port)) {
                        Some(Ok(st)) => { ui.small(format!("{}/{} players · {} · {} ms · {}", st.online, st.max, st.version, st.latency_ms, st.motd.replace('\n', " "))); }
                        Some(Err(e)) => { ui.small(format!("offline: {e}")); }
                        None => {}
                    }
                    if fx_button(ui, fx, "Join", true).clicked() { join = Some(i); }
                    if fx_button(ui, fx, "Status", false).clicked() { ping = Some(i); }
                    if fx_button(ui, fx, "Remove", false).clicked() { remove = Some(i); }
                });
            }
            if let Some(i) = ping { let s = self.servers.servers[i].clone(); self.ping_server(s.host, s.port); }
            if let Some(i) = remove { self.servers.servers.remove(i); let _ = servers::save(&self.dirs, &self.servers); }
            if let Some(i) = join { let s = self.servers.servers[i].clone(); self.play(Some((s.host, s.port))); }
            ui.horizontal_wrapped(|ui| {
                kb_edit(ui, &pf, &mut self.forms.server_name, "Name", 140.0);
                kb_edit(ui, &pf, &mut self.forms.server_addr, "host[:port]", 200.0);
                if fx_button(ui, fx, "Add", false).clicked() {
                    match servers::parse_address(&self.forms.server_addr) {
                        Ok((h, p)) => {
                            let name = if self.forms.server_name.trim().is_empty() { h.clone() } else { self.forms.server_name.trim().to_string() };
                            self.servers.servers.push(ServerEntry { name, host: h, port: p });
                            let _ = servers::save(&self.dirs, &self.servers);
                            self.forms.server_name.clear(); self.forms.server_addr.clear();
                        }
                        Err(e) => self.toast(e.to_string(), true),
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                kb_edit(ui, &pf, &mut self.forms.direct_addr, "Direct connect: host[:port]", 260.0);
                if fx_button(ui, fx, "Connect", true).clicked() {
                    match servers::parse_address(&self.forms.direct_addr) { Ok(a) => self.play(Some(a)), Err(e) => self.toast(e.to_string(), true) }
                }
            });
        });
    }

    // ------------------------------------------------------------------ Profiles
    pub(crate) fn page_profiles(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Profiles");
        panel(ui, fx, |ui| {
            ui.horizontal_wrapped(|ui| {
                kb_edit(ui, &pf, &mut self.forms.new_profile, "New profile name", 220.0);
                if fx_button(ui, fx, "Create", true).clicked() && !self.forms.new_profile.trim().is_empty() {
                    let ver = self.jar_report.as_ref().map(|r| r.version.id.clone()).unwrap_or_default();
                    let p = Profile::new(self.forms.new_profile.trim(), &ver);
                    match self.profiles.add(p) { Ok(id) => { self.profiles.file.selected = Some(id); let _ = self.profiles.save(); self.forms.new_profile.clear(); self.refresh_all(); } Err(e) => self.toast(e.to_string(), true) }
                }
            });
            let mut act: Option<(u8, String)> = None;
            for p in &self.profiles.file.profiles {
                ui.horizontal_wrapped(|ui| {
                    let sel = self.profiles.file.selected.as_deref() == Some(&p.id);
                    if ui.selectable_label(sel, format!("{} ({})", p.name, p.version_id)).clicked() { act = Some((0, p.id.clone())); }
                    if fx_button(ui, fx, "Duplicate", false).clicked() { act = Some((1, p.id.clone())); }
                    if fx_button(ui, fx, "Delete", false).clicked() { act = Some((2, p.id.clone())); }
                });
            }
            match act {
                Some((0, id)) => { self.profiles.file.selected = Some(id); let _ = self.profiles.save(); self.refresh_all(); }
                Some((1, id)) => { if let Err(e) = self.profiles.duplicate(&id) { self.toast(e.to_string(), true); } }
                Some((2, id)) => { // keeps the game folder (worlds, mods) on disk; only the profile entry is removed
                    if let Err(e) = self.profiles.remove(&id, false) { self.toast(e.to_string(), true); } else { self.toast("Profile removed. Its game folder was kept on disk.", false); }
                    self.refresh_all();
                }
                _ => {}
            }
        });
        let Some(id) = self.profiles.file.selected.clone() else { return };
        let Some(mut p) = self.profiles.get(&id).cloned() else { return };
        let before = serde_json::to_string(&p).unwrap_or_default();
        let limits = DeviceLimits { total_ram_mb: self.platform.total_ram_mb() };
        let runtimes: Vec<(Option<String>, String)> = std::iter::once((None, "Automatic".to_string())).chain(self.runtimes.iter().map(|r| (Some(r.name.clone()), format!("{} (Java {})", r.name, r.java_major.map(|m| m.to_string()).unwrap_or("?".into()))))).collect();
        let layouts: Vec<(Option<String>, String)> = std::iter::once((None, "Default".to_string())).chain(self.saved_layouts.iter().map(|l| (Some(l.clone()), l.clone()))).collect();
        let accounts: Vec<(Option<String>, String)> = std::iter::once((None, "Use selected account".to_string())).chain(self.accounts.file.accounts.iter().map(|a| (Some(a.id.clone()), a.username.clone()))).collect();
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong(format!("Settings for {}", p.name));
            ui.horizontal_wrapped(|ui| { ui.label("Name"); kb_edit(ui, &pf, &mut p.name, "name", 220.0); });
            ui.horizontal_wrapped(|ui| {
                ui.label("Minecraft version"); kb_edit(ui, &pf, &mut p.version_id, "26.3", 120.0);
                if let Some(r) = &self.jar_report { if fx_button(ui, fx, "Use imported JAR's", false).clicked() { p.version_id = r.version.id.clone(); } }
            });
            let max = limits.max_heap_mb().max(1024);
            ui.add(egui::Slider::new(&mut p.ram_mb, 512..=max).step_by(128.0).text(format!("RAM (MB) — device {} MB, safe max {}", limits.total_ram_mb, max)));
            ui.add(egui::Slider::new(&mut p.render_scale, 0.25..=1.5).text("Render scale"));
            ui.horizontal(|ui| { ui.label("Resolution (0 = automatic)"); ui.add(egui::DragValue::new(&mut p.width).range(0..=7680)); ui.label("×"); ui.add(egui::DragValue::new(&mut p.height).range(0..=4320)); });
            let mut preset = p.perf_preset;
            combo(ui, "Performance preset", &mut preset, &[(PerfPreset::Battery, "Battery saver"), (PerfPreset::Balanced, "Balanced"), (PerfPreset::Quality, "Quality")]);
            if preset != p.perf_preset { p.apply_preset(preset); }
            combo(ui, "Graphics backend", &mut p.graphics_backend, &[(GraphicsBackend::Default, "Game default"), (GraphicsBackend::OpenGl, "OpenGL"), (GraphicsBackend::Vulkan, "Vulkan")]);
            combo(ui, "Mod loader", &mut p.loader, &[(Loader::Vanilla, "None (vanilla)"), (Loader::Fabric, "Fabric"), (Loader::Quilt, "Quilt"), (Loader::Forge, "Forge"), (Loader::NeoForge, "NeoForge")]);
            combo(ui, "Java runtime", &mut p.runtime, &runtimes.iter().map(|(a, b)| (a.clone(), b.as_str())).collect::<Vec<_>>());
            combo(ui, "Controls layout", &mut p.layout_id, &layouts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect::<Vec<_>>());
            combo(ui, "Account", &mut p.account_id, &accounts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect::<Vec<_>>());
            ui.horizontal_wrapped(|ui| { ui.label("JVM args"); kb_edit(ui, &pf, &mut p.jvm_args, "-XX:… (no -Xmx/-Xms)", 360.0); });
            ui.horizontal_wrapped(|ui| { ui.label("Game args"); kb_edit(ui, &pf, &mut p.game_args, "extra game arguments", 360.0); });
            let issues = validate(&p, limits);
            if issues.is_empty() { ui.colored_label(Color32::from_rgb(90, 200, 120), "Configuration is valid."); }
            for i in issues { ui.colored_label(if i.error { Color32::from_rgb(235, 90, 90) } else { Color32::from_rgb(230, 190, 80) }, i.message); }
        });
        if serde_json::to_string(&p).unwrap_or_default() != before {
            if let Some(slot) = self.profiles.get_mut(&id) { *slot = p; }
            let _ = self.profiles.save();
            self.refresh_all();
        }
    }

    // ------------------------------------------------------------------ Accounts
    pub(crate) fn page_accounts(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Accounts");
        panel(ui, fx, |ui| {
            ui.strong("Offline profile");
            ui.small("Offline profiles are not authenticated. They work only on singleplayer and on servers that explicitly allow offline players.");
            ui.horizontal_wrapped(|ui| {
                kb_edit(ui, &pf, &mut self.forms.offline_name, "Player name (3–16)", 200.0);
                if fx_button(ui, fx, "Add offline profile", true).clicked() {
                    match self.accounts.add_offline(self.forms.offline_name.trim()) { Ok(_) => self.forms.offline_name.clear(), Err(e) => self.toast(e.to_string(), true) }
                }
            });
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong("Microsoft / Minecraft account");
            ui.small("Sign-in uses Microsoft's device-code flow. You need an Azure application (client) ID that Mojang has approved for the Minecraft services API. Tokens are kept in Android Keystore-backed storage and masked in logs.");
            ui.horizontal_wrapped(|ui| {
                ui.label("Client ID");
                if kb_edit(ui, &pf, &mut self.settings.microsoft_client_id, "xxxxxxxx-xxxx-…", 340.0).changed() { self.settings_dirty = true; }
            });
            if let Some(dc) = self.ms.dc.clone() {
                ui.label("Open the page below and enter this code:");
                ui.heading(&dc.user_code);
                ui.horizontal(|ui| {
                    if fx_button(ui, fx, "Open sign-in page", true).clicked() { pf.open_url(&dc.verification_uri); }
                    if fx_button(ui, fx, "Cancel", false).clicked() { self.ms.cancel.store(true, std::sync::atomic::Ordering::Relaxed); }
                });
                ui.small(&dc.verification_uri);
            } else if self.ms.working { ui.label("Contacting Microsoft…"); }
            else if fx_button(ui, fx, "Sign in with Microsoft", true).clicked() { self.start_microsoft_login(); }
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong("Saved accounts");
            let mut act: Option<(bool, String)> = None;
            for a in &self.accounts.file.accounts {
                ui.horizontal_wrapped(|ui| {
                    let sel = self.accounts.file.selected.as_deref() == Some(&a.id);
                    let kind = if a.kind == launcher_core::accounts::AccountKind::Offline { "OFFLINE" } else { "MICROSOFT" };
                    if ui.selectable_label(sel, format!("[{kind}] {}  {}", a.username, a.uuid)).clicked() { act = Some((true, a.id.clone())); }
                    if fx_button(ui, fx, "Remove", false).clicked() { act = Some((false, a.id.clone())); }
                });
            }
            match act {
                Some((true, id)) => { self.accounts.file.selected = Some(id); let _ = self.accounts.save(); }
                Some((false, id)) => { let s = self.platform.secrets(); if let Err(e) = self.accounts.remove(&id, &*s) { self.toast(e.to_string(), true); } }
                None => {}
            }
        });
    }

    // ------------------------------------------------------------------ Mods
    pub(crate) fn page_mods(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Mods");
        let Some(p) = self.profiles.selected().cloned() else { ui.label("Create a profile first."); return };
        panel(ui, fx, |ui| {
            ui.label(format!("Profile: {} · loader: {:?} · Minecraft {}", p.name, p.loader, p.version_id));
            ui.small("Forge, NeoForge and Fabric/Quilt mods are not interchangeable. Mods that bundle desktop native libraries (.so/.dll/.dylib) or need desktop-only features may crash or be unsupported on Android. Compatibility is not guaranteed.");
            if fx_button(ui, fx, "Import mod (.jar)", true).clicked() { pf.pick_file("mod"); }
        });
        let issues = launcher_core::mods::analyze(&self.mods, p.loader, &p.version_id);
        if !issues.is_empty() {
            ui.add_space(6.0);
            panel(ui, fx, |ui| { for i in &issues { ui.colored_label(if i.error { Color32::from_rgb(235, 90, 90) } else { Color32::from_rgb(230, 190, 80) }, &i.message); } });
        }
        ui.add_space(6.0);
        let mut toggle: Option<(std::path::PathBuf, bool)> = None;
        let mut remove: Option<std::path::PathBuf> = None;
        panel(ui, fx, |ui| {
            if self.mods.is_empty() { ui.label("No mods in this profile."); }
            for m in &self.mods {
                ui.horizontal_wrapped(|ui| {
                    let mut en = m.enabled;
                    if ui.checkbox(&mut en, "").changed() { toggle = Some((m.file.clone(), en)); }
                    ui.label(format!("{}{}{}", m.name, m.version.as_ref().map(|v| format!(" {v}")).unwrap_or_default(), m.loader.map(|l| format!("  [{l:?}]")).unwrap_or_default()));
                    if fx_button(ui, fx, "Remove", false).clicked() { remove = Some(m.file.clone()); }
                });
            }
        });
        if let Some((f, en)) = toggle { if let Err(e) = launcher_core::mods::set_enabled(&f, en) { self.toast(e.to_string(), true); } self.refresh_all(); }
        if let Some(f) = remove { if let Err(e) = launcher_core::mods::remove(&f) { self.toast(e.to_string(), true); } self.refresh_all(); }
    }

    // ------------------------------------------------------------------ Resource packs
    pub(crate) fn page_resource_packs(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Resource Packs");
        let Some(inst) = self.profiles.selected().map(|p| self.dirs.instance(&p.id)) else { ui.label("Create a profile first."); return };
        panel(ui, fx, |ui| {
            ui.small("Order matches the in-game list: the last entry has the highest priority. Changes are written to this profile's options.txt, so the game shows the same selection.");
            if fx_button(ui, fx, "Import resource pack (.zip)", true).clicked() { pf.pick_file("resourcepack"); }
        });
        ui.add_space(6.0);
        let mut enabled = self.rp_enabled.clone();
        let mut changed = false;
        panel(ui, fx, |ui| {
            ui.strong("Enabled");
            let mut mv: Option<(usize, i32)> = None; let mut off: Option<usize> = None;
            for (i, e) in enabled.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(e.strip_prefix("file/").unwrap_or(e));
                    if fx_button(ui, fx, "Up", false).clicked() { mv = Some((i, -1)); }
                    if fx_button(ui, fx, "Down", false).clicked() { mv = Some((i, 1)); }
                    if e != "vanilla" && fx_button(ui, fx, "Disable", false).clicked() { off = Some(i); }
                });
            }
            if let Some((i, d)) = mv { let j = i as i32 + d; if j >= 0 && (j as usize) < enabled.len() { enabled.swap(i, j as usize); changed = true; } }
            if let Some(i) = off { enabled.remove(i); changed = true; }
            ui.separator();
            ui.strong("Available");
            let mut rm: Option<String> = None;
            for f in &self.rp_files {
                let entry = packs::pack_entry_for_file(f);
                if enabled.contains(&entry) { continue; }
                ui.horizontal_wrapped(|ui| {
                    ui.label(f);
                    if fx_button(ui, fx, "Enable", true).clicked() { enabled.push(entry.clone()); changed = true; }
                    if fx_button(ui, fx, "Delete", false).clicked() { rm = Some(f.clone()); }
                });
            }
            if let Some(f) = rm { let _ = packs::remove_file(&inst.join("resourcepacks"), &f); changed = true; }
        });
        if changed {
            let existing: Vec<String> = self.rp_files.iter().map(|f| packs::pack_entry_for_file(f)).collect();
            enabled.retain(|e| e == "vanilla" || !e.starts_with("file/") || existing.contains(e) && inst.join("resourcepacks").join(e.trim_start_matches("file/")).exists());
            if let Err(e) = packs::set_enabled_resource_packs(&inst, &enabled) { self.toast(e.to_string(), true); }
            self.refresh_all();
        }
    }

    // ------------------------------------------------------------------ Shaders
    pub(crate) fn page_shaders(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Shaders");
        let Some(p) = self.profiles.selected().cloned() else { ui.label("Create a profile first."); return };
        let inst = self.dirs.instance(&p.id);
        let has_iris = self.mods.iter().any(|m| m.enabled && m.id.as_deref() == Some("iris"));
        panel(ui, fx, |ui| {
            ui.small("Shader packs need a shader mod (Iris on Fabric/Quilt, or OptiFine-compatible setups). They are not part of vanilla Minecraft. Support depends on the device GPU, the graphics backend (OpenGL translation vs Vulkan), the Minecraft version and the loader; many packs will be slow or fail on mobile GPUs. No compatibility is promised.");
            if !has_iris { ui.colored_label(Color32::from_rgb(230, 190, 80), "Iris was not found among this profile's enabled mods, so selecting a pack has no effect yet."); }
            if fx_button(ui, fx, "Import shader pack (.zip)", true).clicked() { pf.pick_file("shaderpack"); }
        });
        ui.add_space(6.0);
        let mut act: Option<(u8, String)> = None;
        panel(ui, fx, |ui| {
            if self.shader_files.is_empty() { ui.label("No shader packs imported."); }
            for f in &self.shader_files {
                ui.horizontal_wrapped(|ui| {
                    ui.label(f);
                    if fx_button(ui, fx, "Use", true).clicked() { act = Some((0, f.clone())); }
                    if fx_button(ui, fx, "Delete", false).clicked() { act = Some((1, f.clone())); }
                });
            }
            if fx_button(ui, fx, "Disable shaders", false).clicked() { act = Some((2, String::new())); }
        });
        match act {
            Some((0, f)) => { match packs::set_iris_shader(&inst, Some(&f)) { Ok(_) => self.toast(format!("Shader pack '{f}' selected (applies through Iris)."), false), Err(e) => self.toast(e.to_string(), true) } }
            Some((1, f)) => { let _ = packs::remove_file(&inst.join("shaderpacks"), &f); self.refresh_all(); }
            Some((2, _)) => { let _ = packs::set_iris_shader(&inst, None); self.toast("Shaders disabled.", false); }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ Controls
    pub(crate) fn page_controls(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Controls");
        panel(ui, fx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut cur = self.layout_slug.clone();
                egui::ComboBox::from_label("Layout").selected_text(&cur).show_ui(ui, |ui| {
                    ui.selectable_value(&mut cur, "default".to_string(), "default (built-in)");
                    for l in &self.saved_layouts { ui.selectable_value(&mut cur, l.clone(), l); }
                });
                if cur != self.layout_slug {
                    if cur == "default" { self.layout = controls::default_layout(); } else if let Ok(l) = controls::load_layout(&self.dirs, &cur) { self.layout = l; }
                    self.layout_slug = cur; self.sel = None;
                }
                kb_edit(ui, &pf, &mut self.forms.layout_name, "Save as…", 160.0);
                if fx_button(ui, fx, "Save", true).clicked() {
                    if !self.forms.layout_name.trim().is_empty() { self.layout.name = self.forms.layout_name.trim().to_string(); }
                    if self.layout.name == "Default" && self.layout_slug == "default" { self.toast("Enter a name in 'Save as…' to save a custom layout.", true); }
                    else { match controls::save_layout(&self.dirs, &self.layout) { Ok(s) => { self.layout_slug = s; self.toast("Layout saved.", false); self.refresh_all(); } Err(e) => self.toast(e.to_string(), true) } }
                }
                if fx_button(ui, fx, "Reset", false).clicked() { self.layout = controls::default_layout(); self.sel = None; }
                if fx_button(ui, fx, "Delete", false).clicked() && self.layout_slug != "default" {
                    let _ = controls::delete_layout(&self.dirs, &self.layout_slug); self.layout_slug = "default".into(); self.layout = controls::default_layout(); self.refresh_all();
                }
            });
            ui.horizontal_wrapped(|ui| {
                if fx_button(ui, fx, "Import", false).clicked() { pf.pick_file("layout"); }
                if fx_button(ui, fx, "Export", false).clicked() {
                    if let Ok(j) = self.layout.to_json() {
                        let path = self.dirs.imports().join("export_layout.json");
                        if std::fs::write(&path, j).is_ok() { pf.export_file(&path, &format!("{}.json", controls::layout_slug(&self.layout.name)), "application/json"); }
                    }
                }
                if fx_button(ui, fx, "Use for current profile", false).clicked() {
                    if self.layout_slug == "default" { self.toast("Save the layout under a name first.", true); }
                    else if let Some(id) = self.profiles.file.selected.clone() { if let Some(p) = self.profiles.get_mut(&id) { p.layout_id = Some(self.layout_slug.clone()); } let _ = self.profiles.save(); self.toast("Layout assigned to the profile.", false); }
                }
                ui.checkbox(&mut self.snap_on, "Snap & guides");
            });
            for m in self.layout.validate() { ui.colored_label(Color32::from_rgb(230, 190, 80), m); }
        });
        ui.add_space(6.0);

        let (ux, uy) = self.platform.usable_size();
        let aspect = (ux.max(1) as f32) / (uy.max(1) as f32);
        let mut w = (ui.available_width() * 0.62).min(900.0);
        let mut h = w / aspect;
        if h > 330.0 { h = 330.0; w = h * aspect; }
        ui.horizontal_top(|ui| {
            let (canvas, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            let painter = ui.painter_at(canvas);
            painter.rect_filled(canvas, CornerRadius::same(6), Color32::from_black_alpha(190));
            painter.line_segment([pos2(canvas.center().x, canvas.top()), pos2(canvas.center().x, canvas.bottom())], Stroke::new(1.0, Color32::from_white_alpha(25)));
            painter.line_segment([pos2(canvas.left(), canvas.center().y), pos2(canvas.right(), canvas.center().y)], Stroke::new(1.0, Color32::from_white_alpha(25)));
            if !ui.input(|i| i.pointer.any_down()) { self.guides.clear(); }
            let n = self.layout.elements.len();
            let to_screen = |r: &controls::Rect| Rect::from_min_size(canvas.min + vec2(r.x * w, r.y * h), vec2(r.w * w, r.h * h));
            for i in 0..n {
                let el = self.layout.elements[i].clone();
                let er = to_screen(&el.rect);
                let a = el.opacity.max(0.14) * if el.visible { 1.0 } else { 0.3 };
                painter.rect_filled(er, CornerRadius::same(8), fx.accent.gamma_multiply(a));
                match &el.kind {
                    ElementKind::Joystick { .. } => { painter.circle_stroke(er.center(), er.height().min(er.width()) * 0.35, Stroke::new(2.0, Color32::WHITE)); }
                    ElementKind::Hotbar { slots } => { for s in 1..*slots { let x = er.left() + er.width() * s as f32 / *slots as f32; painter.line_segment([pos2(x, er.top()), pos2(x, er.bottom())], Stroke::new(1.0, Color32::WHITE)); } }
                    _ => {}
                }
                painter.text(er.center(), egui::Align2::CENTER_CENTER, &el.label, egui::FontId::proportional(12.0), Color32::WHITE);
                if self.sel == Some(i) { painter.rect_stroke(er, CornerRadius::same(8), Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside); }
                let resp = ui.interact(er, egui::Id::new(("ctl_el", i)), Sense::click_and_drag());
                if resp.clicked() || resp.drag_started() { self.sel = Some(i); }
                if resp.dragged() {
                    let d = resp.drag_delta();
                    let mut nr = controls::Rect { x: el.rect.x + d.x / w, y: el.rect.y + d.y / h, ..el.rect };
                    if self.snap_on {
                        let others: Vec<controls::Rect> = self.layout.elements.iter().enumerate().filter(|(j, e)| *j != i && e.visible).map(|(_, e)| e.rect).collect();
                        let (s, g) = controls::snap(nr, &others, 0.012);
                        nr = s; self.guides = g;
                    }
                    self.layout.elements[i].rect = nr;
                }
            }
            if let Some(i) = self.sel.filter(|i| *i < n) {
                let er = to_screen(&self.layout.elements[i].rect);
                let hr = Rect::from_center_size(er.right_bottom(), vec2(24.0, 24.0));
                painter.rect_filled(hr, CornerRadius::same(4), Color32::WHITE);
                let hresp = ui.interact(hr, egui::Id::new("ctl_handle"), Sense::drag());
                if hresp.dragged() { let d = hresp.drag_delta(); let r = &mut self.layout.elements[i].rect; r.w += d.x / w; r.h += d.y / h; }
            }
            for g in &self.guides {
                let (a, b) = if g.vertical { (pos2(canvas.min.x + g.pos * w, canvas.top()), pos2(canvas.min.x + g.pos * w, canvas.bottom())) } else { (pos2(canvas.left(), canvas.min.y + g.pos * h), pos2(canvas.right(), canvas.min.y + g.pos * h)) };
                painter.line_segment([a, b], Stroke::new(1.0, Color32::YELLOW));
            }
            self.layout.clamp();

            ui.vertical(|ui| {
                let Some(i) = self.sel.filter(|i| *i < self.layout.elements.len()) else {
                    ui.label("Tap a control to edit it. Drag to move; drag the white square to resize.");
                    return;
                };
                let mut el: Element = self.layout.elements[i].clone();
                ui.horizontal_wrapped(|ui| { ui.label("Label"); kb_edit(ui, &pf, &mut el.label, "label", 120.0); });
                ui.add(egui::Slider::new(&mut el.opacity, 0.0..=1.0).text("Opacity"));
                ui.checkbox(&mut el.visible, "Visible");
                match &mut el.kind {
                    ElementKind::Button { binding, mode } => {
                        let mut k = binding_kind(binding);
                        combo(ui, "Input", &mut k, &[(0, "Keyboard key"), (1, "Mouse left"), (2, "Mouse right"), (3, "Mouse middle"), (4, "Scroll up"), (5, "Scroll down")]);
                        if k != binding_kind(binding) {
                            *binding = match k { 0 => Binding::Key("F".into()), 1 => Binding::Mouse(MouseButton::Left), 2 => Binding::Mouse(MouseButton::Right), 3 => Binding::Mouse(MouseButton::Middle), 4 => Binding::ScrollUp, _ => Binding::ScrollDown };
                        }
                        if let Binding::Key(key) = binding {
                            egui::ComboBox::from_label("Key").selected_text(key.as_str()).show_ui(ui, |ui| { for kname in KEYS { ui.selectable_value(key, kname.to_string(), *kname); } });
                            ui.horizontal_wrapped(|ui| { ui.label("Custom key"); if kb_edit(ui, &pf, &mut self.forms.custom_key, "e.g. F6, KP_1", 100.0).lost_focus() && !self.forms.custom_key.trim().is_empty() { *key = self.forms.custom_key.trim().to_uppercase(); self.forms.custom_key.clear(); } });
                        }
                        combo(ui, "Action", mode, &[(Mode::Hold, "Hold"), (Mode::Toggle, "Toggle")]);
                    }
                    ElementKind::Camera { sensitivity } => { ui.add(egui::Slider::new(sensitivity, 0.1..=5.0).text("Camera sensitivity")); }
                    ElementKind::Joystick { deadzone, sprint_threshold } => {
                        ui.add(egui::Slider::new(deadzone, 0.05..=0.6).text("Dead zone"));
                        ui.add(egui::Slider::new(sprint_threshold, 0.5..=1.0).text("Auto-sprint distance"));
                    }
                    ElementKind::Hotbar { slots } => { ui.add(egui::Slider::new(slots, 1..=9).text("Slots")); }
                }
                self.layout.elements[i] = el;
                ui.horizontal_wrapped(|ui| {
                    if fx_button(ui, fx, "Forward", false).clicked() && i + 1 < self.layout.elements.len() { self.layout.elements.swap(i, i + 1); self.sel = Some(i + 1); }
                    if fx_button(ui, fx, "Back", false).clicked() && i > 0 { self.layout.elements.swap(i, i - 1); self.sel = Some(i - 1); }
                    if fx_button(ui, fx, "Delete", false).clicked() { self.layout.elements.remove(i); self.sel = None; }
                });
            });
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Add:");
                let mut add = |e: Element, this: &mut App| { this.layout.elements.push(e); this.sel = Some(this.layout.elements.len() - 1); };
                let r = controls::Rect { x: 0.45, y: 0.4, w: 0.1, h: 0.2 };
                if fx_button(ui, fx, "Button", false).clicked() { add(Element { id: format!("btn{}", self.layout.elements.len()), label: "Btn".into(), kind: ElementKind::Button { binding: Binding::Key("F".into()), mode: Mode::Hold }, rect: r, opacity: 0.6, visible: true }, self); }
                if fx_button(ui, fx, "Joystick", false).clicked() { add(Element { id: format!("move{}", self.layout.elements.len()), label: "Move".into(), kind: ElementKind::Joystick { deadzone: 0.25, sprint_threshold: 0.9 }, rect: controls::Rect { x: 0.05, y: 0.5, w: 0.17, h: 0.44 }, opacity: 0.55, visible: true }, self); }
                if fx_button(ui, fx, "Camera region", false).clicked() { add(Element { id: format!("cam{}", self.layout.elements.len()), label: "Look".into(), kind: ElementKind::Camera { sensitivity: 1.0 }, rect: controls::Rect { x: 0.35, y: 0.0, w: 0.65, h: 1.0 }, opacity: 0.0, visible: true }, self); }
                if fx_button(ui, fx, "Hotbar", false).clicked() { add(Element { id: format!("hotbar{}", self.layout.elements.len()), label: "Hotbar".into(), kind: ElementKind::Hotbar { slots: 9 }, rect: controls::Rect { x: 0.36, y: 0.88, w: 0.3, h: 0.11 }, opacity: 0.45, visible: true }, self); }
            });
            ui.small("External keyboards, mice and controllers are forwarded in-game as well: controller sticks move/look, A jump, B sneak, triggers attack/use, bumpers scroll the hotbar, Y inventory, Start pause.");
        });
    }

    // ------------------------------------------------------------------ Settings
    pub(crate) fn page_settings(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Settings");
        panel(ui, fx, |ui| {
            ui.strong("Appearance");
            ui.horizontal_wrapped(|ui| {
                for t in Theme::presets() { if fx_button(ui, fx, &t.name, false).clicked() { self.settings.theme = t; self.settings_dirty = true; self.theme_dirty = true; } }
            });
            let mut ch = false;
            let t = &mut self.settings.theme;
            ui.horizontal_wrapped(|ui| {
                ch |= egui::color_picker::color_edit_button_srgb(ui, &mut t.accent).changed(); ui.label("Accent");
                ch |= egui::color_picker::color_edit_button_srgb(ui, &mut t.background).changed(); ui.label("Background");
                ch |= egui::color_picker::color_edit_button_srgb(ui, &mut t.panel).changed(); ui.label("Panels");
                ch |= egui::color_picker::color_edit_button_srgb(ui, &mut t.text).changed(); ui.label("Text");
            });
            ch |= ui.checkbox(&mut t.dark, "Dark base").changed();
            ch |= ui.add(egui::Slider::new(&mut t.panel_opacity, 0.2..=1.0).text("Panel opacity")).changed();
            ch |= ui.add(egui::Slider::new(&mut t.corner_radius, 0.0..=28.0).text("Corner radius")).changed();
            ch |= ui.add(egui::Slider::new(&mut t.font_scale, 0.8..=1.5).text("Text size")).changed();
            let mut f = t.font;
            combo(ui, "Typeface", &mut f, &[(FontChoice::Proportional, "Sans (default)"), (FontChoice::Monospace, "Monospace")]);
            if f != t.font { t.font = f; ch = true; }
            ui.horizontal_wrapped(|ui| {
                if fx_button(ui, fx, "Import background", false).clicked() { pf.pick_file("background"); }
                if t.background_image.is_some() && fx_button(ui, fx, "Clear background", false).clicked() { t.background_image = None; ch = true; }
            });
            ui.small("Backgrounds: PNG, JPEG, WebP or animated GIF. Animated images are decoded continuously; disable background animation to save battery.");
            if ch { self.settings_dirty = true; self.theme_dirty = true; }
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong("Motion & scale");
            let mut ch = false;
            ch |= ui.add(egui::Slider::new(&mut self.settings.ui_scale, 0.7..=1.8).text("Interface scale")).changed();
            ch |= ui.add(egui::Slider::new(&mut self.settings.anim_speed, 0.25..=2.0).text("Animation speed")).changed();
            ch |= ui.checkbox(&mut self.settings.reduced_motion, "Reduced motion (instant transitions)").changed();
            ch |= ui.checkbox(&mut self.settings.background_animation, "Animated background").changed();
            if ch { self.settings_dirty = true; self.theme_dirty = true; }
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong("Saved themes");
            ui.horizontal_wrapped(|ui| {
                kb_edit(ui, &pf, &mut self.forms.theme_name, "Theme name", 180.0);
                if fx_button(ui, fx, "Save current", true).clicked() && !self.forms.theme_name.trim().is_empty() {
                    let mut t = self.settings.theme.clone(); t.name = self.forms.theme_name.trim().to_string(); t.clamp();
                    match save_theme(&self.dirs, &t) { Ok(_) => { self.settings.theme.name = t.name; self.refresh_all(); } Err(e) => self.toast(e.to_string(), true) }
                }
                if fx_button(ui, fx, "Import", false).clicked() { pf.pick_file("theme"); }
                if fx_button(ui, fx, "Export current", false).clicked() {
                    if let Ok(j) = self.settings.theme.to_json() { let p = self.dirs.imports().join("export_theme.json"); if std::fs::write(&p, j).is_ok() { pf.export_file(&p, &format!("{}.json", controls::layout_slug(&self.settings.theme.name)), "application/json"); } }
                }
                if fx_button(ui, fx, "Reset to default", false).clicked() { self.settings.theme = Theme::default(); self.settings.ui_scale = 1.0; self.settings.anim_speed = 1.0; self.settings.reduced_motion = false; self.settings.background_animation = true; self.settings_dirty = true; self.theme_dirty = true; }
            });
            let mut act: Option<(bool, Theme)> = None;
            for t in &self.saved_themes { ui.horizontal_wrapped(|ui| { ui.label(&t.name); if fx_button(ui, fx, "Apply", false).clicked() { act = Some((true, t.clone())); } if fx_button(ui, fx, "Delete", false).clicked() { act = Some((false, t.clone())); } }); }
            match act { Some((true, t)) => { self.settings.theme = t; self.settings_dirty = true; self.theme_dirty = true; } Some((false, t)) => { let _ = delete_theme(&self.dirs, &t.name); self.refresh_all(); } None => {} }
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong("Java runtimes & native libraries");
            ui.small("The game needs a Java runtime built for Android arm64 (bionic). Desktop JDKs will not load. Import a .zip or .tar.gz that contains lib/server/libjvm.so.");
            for r in &self.runtimes { ui.label(format!("• {} — Java {}", r.name, r.java_major.map(|m| m.to_string()).unwrap_or("?".into()))); }
            ui.horizontal_wrapped(|ui| {
                if fx_button(ui, fx, "Import Java runtime", true).clicked() { pf.pick_file("runtime"); }
                if fx_button(ui, fx, "Add LWJGL (.jar / .so)", false).clicked() { pf.pick_file("lwjgl"); }
            });
            ui.small("Add an Android-compatible LWJGL build (with SDL support for this Minecraft version). When present, it replaces the desktop LWJGL libraries listed by Mojang.");
        });
    }

    // ------------------------------------------------------------------ Logs
    pub(crate) fn page_logs(&mut self, ui: &mut Ui) {
        let (fx, pf) = (self.fx, self.platform.clone());
        ui.heading("Logs");
        panel(ui, fx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if fx_button(ui, fx, "Load game log", false).clicked() {
                    self.game_log_tail = std::fs::read_to_string(self.dirs.logs().join("game-process.log")).map(|t| { let v: Vec<&str> = t.lines().collect(); logs::redact(&v[v.len().saturating_sub(300)..].join("\n")) }).unwrap_or_else(|_| "No game log yet.".into());
                }
                if fx_button(ui, fx, "Clear launcher log", false).clicked() { logs::clear(); }
                if fx_button(ui, fx, "Export logs (ZIP)", true).clicked() {
                    let inst = self.profiles.selected().map(|p| self.dirs.instance(&p.id));
                    let out = self.dirs.imports().join("rustcraft-logs.zip");
                    match logs::export_bundle(&self.dirs, inst.as_deref(), &out) { Ok(_) => pf.export_file(&out, "rustcraft-logs.zip", "application/zip"), Err(e) => self.toast(e.to_string(), true) }
                }
            });
        });
        ui.add_space(6.0);
        panel(ui, fx, |ui| {
            ui.strong("Launcher");
            egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).id_salt("launcher_log").show(ui, |ui| { ui.add(egui::Label::new(egui::RichText::new(logs::snapshot().join("\n")).monospace().size(12.0)).selectable(true)); });
            if !self.game_log_tail.is_empty() {
                ui.strong("Game process");
                egui::ScrollArea::vertical().max_height(220.0).id_salt("game_log").show(ui, |ui| { ui.add(egui::Label::new(egui::RichText::new(&self.game_log_tail).monospace().size(12.0)).selectable(true)); });
            }
        });
    }
}
