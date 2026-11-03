//! Builds the launch plan consumed by the native JVM host.
use crate::error::{Error, Result};
use crate::manifest::*;
use crate::paths::Dirs;
use crate::profiles::{split_args, Profile};
use crate::runtime::RuntimeInfo;
use crate::store::write_private;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub const LAUNCHER_NAME: &str = "Rustcraft";

#[derive(Debug, Clone)]
pub struct LaunchAccount {
    pub username: String,
    pub uuid: String,
    /// Real Minecraft access token for Microsoft accounts; "0" for offline profiles.
    pub access_token: String,
    pub microsoft: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchPlan {
    pub java_home: PathBuf,
    pub libjvm: PathBuf,
    pub jvm_args: Vec<String>,
    pub main_class: String,
    pub game_args: Vec<String>,
    pub working_dir: PathBuf,
    pub env: BTreeMap<String, String>,
    pub log_file: PathBuf,
    pub width: u32,
    pub height: u32,
    pub render_scale: f32,
    /// Controls layout (JSON) the game activity should load for this profile.
    pub layout_file: Option<PathBuf>,
    /// Argument values that must never be logged.
    pub secrets: Vec<String>,
}

pub struct LaunchInputs<'a> {
    pub dirs: &'a Dirs,
    pub profile: &'a Profile,
    pub manifest: &'a VersionManifest,
    pub account: &'a LaunchAccount,
    pub runtime: &'a RuntimeInfo,
    /// Screen size in pixels (landscape) used when the profile resolution is automatic.
    pub screen: (u32, u32),
    /// Optional direct connect target.
    pub quick_play: Option<(String, u16)>,
}

fn substitute(arg: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = arg.to_string();
    for (k, v) in vars { out = out.replace(&format!("${{{k}}}"), v); }
    out
}

pub fn build_plan(i: &LaunchInputs) -> Result<LaunchPlan> {
    let id = &i.manifest.id;
    let client_jar = i.dirs.version_dir(id).join(format!("{id}.jar"));
    if !client_jar.is_file() { return Err(Error::invalid("Client JAR is not installed. Run game preparation first.")); }
    let instance = i.dirs.instance(&i.profile.id);
    std::fs::create_dir_all(&instance)?;
    let natives = i.dirs.root.join("natives").join(id);
    std::fs::create_dir_all(&natives)?;
    let tmp = i.dirs.root.join("run").join("tmp");
    std::fs::create_dir_all(&tmp)?;

    // Classpath. Desktop LWJGL builds do not run on Android, so when an Android LWJGL set is
    // installed it replaces the manifest's org.lwjgl.* libraries.
    let lwjgl_dir = i.dirs.lwjgl_override();
    let lwjgl_jars: Vec<PathBuf> = std::fs::read_dir(&lwjgl_dir).map(|rd| {
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "jar")).collect();
        v.sort(); v
    }).unwrap_or_default();
    let use_override = !lwjgl_jars.is_empty();
    let env = crate::prepare::library_env();
    let mut cp: Vec<String> = Vec::new();
    for l in i.manifest.libraries.iter().filter(|l| rules_allow(&l.rules, &env)) {
        if use_override && l.name.starts_with("org.lwjgl") { continue; }
        if let Some(a) = l.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            cp.push(i.dirs.libraries().join(&a.path).display().to_string());
        }
    }
    cp.extend(lwjgl_jars.iter().map(|p| p.display().to_string()));
    cp.push(client_jar.display().to_string());

    let (sw, sh) = i.screen;
    let w = if i.profile.width > 0 { i.profile.width } else { ((sw as f32) * i.profile.render_scale) as u32 };
    let h = if i.profile.height > 0 { i.profile.height } else { ((sh as f32) * i.profile.render_scale) as u32 };

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("auth_player_name", i.account.username.clone());
    vars.insert("version_name", id.clone());
    vars.insert("game_directory", instance.display().to_string());
    vars.insert("assets_root", i.dirs.assets().display().to_string());
    vars.insert("assets_index_name", i.manifest.asset_index.id.clone());
    vars.insert("auth_uuid", i.account.uuid.clone());
    vars.insert("auth_access_token", i.account.access_token.clone());
    vars.insert("clientid", "0".into());
    vars.insert("auth_xuid", "0".into());
    vars.insert("user_type", if i.account.microsoft { "msa" } else { "legacy" }.into());
    vars.insert("version_type", i.manifest.kind.clone());
    vars.insert("resolution_width", w.to_string());
    vars.insert("resolution_height", h.to_string());
    vars.insert("natives_directory", natives.display().to_string());
    vars.insert("launcher_name", LAUNCHER_NAME.into());
    vars.insert("launcher_version", env!("CARGO_PKG_VERSION").into());
    vars.insert("classpath", cp.join(":"));
    vars.insert("classpath_separator", ":".into());
    vars.insert("library_directory", i.dirs.libraries().display().to_string());
    if let Some((h, p)) = &i.quick_play { vars.insert("quickPlayMultiplayer", format!("{h}:{p}")); }

    let rule_env = RuleEnv::android_arm64(&[
        ("has_custom_resolution", true),
        ("is_demo_user", false),
        ("has_quick_plays_support", true),
        ("is_quick_play_multiplayer", i.quick_play.is_some()),
        ("is_quick_play_singleplayer", false),
        ("is_quick_play_realms", false),
    ]);

    let args = i.manifest.arguments.clone().unwrap_or_default();
    let mut jvm: Vec<String> = if args.jvm.is_empty() {
        vec!["-Djava.library.path=${natives_directory}".into(), "-cp".into(), "${classpath}".into()]
    } else {
        expand_arguments(&args.jvm, &rule_env)
    };
    let mut game: Vec<String> = if let Some(legacy) = &i.manifest.minecraft_arguments {
        legacy.split_whitespace().map(str::to_string).collect()
    } else {
        expand_arguments(&args.game, &rule_env)
    };

    // Android-specific and profile JVM settings.
    let mut extra = vec![
        format!("-Xms{}m", (i.profile.ram_mb / 4).max(256)),
        format!("-Xmx{}m", i.profile.ram_mb),
        format!("-Djava.io.tmpdir={}", tmp.display()),
        format!("-Duser.home={}", instance.display()),
        "-Dfile.encoding=UTF-8".into(),
    ];
    if use_override { extra.push(format!("-Dorg.lwjgl.librarypath={}", lwjgl_dir.display())); }
    extra.extend(split_args(&i.profile.jvm_args));
    // Insert before the classpath switch so options precede the main class either way.
    jvm.splice(0..0, extra);

    if let Some(v) = i.profile.graphics_backend.cli_value() {
        game.push("--graphicsBackend".into());
        game.push(v.into());
    }
    game.extend(split_args(&i.profile.game_args));

    let jvm: Vec<String> = jvm.iter().map(|a| substitute(a, &vars)).collect();
    let game: Vec<String> = game.iter().map(|a| substitute(a, &vars)).collect();
    if let Some(bad) = jvm.iter().chain(game.iter()).find(|a| a.contains("${")) {
        return Err(Error::invalid(format!("Unresolved launch placeholder in argument '{bad}'.")));
    }

    let mut envm = BTreeMap::new();
    envm.insert("HOME".into(), instance.display().to_string());
    envm.insert("TMPDIR".into(), tmp.display().to_string());
    envm.insert("LD_LIBRARY_PATH".into(), [
        i.runtime.libjvm.parent().map(|p| p.display().to_string()).unwrap_or_default(),
        i.runtime.home.join("lib").display().to_string(),
        lwjgl_dir.display().to_string(),
    ].join(":"));

    let mut secrets = Vec::new();
    if i.account.microsoft { secrets.push(i.account.access_token.clone()); }

    Ok(LaunchPlan {
        java_home: i.runtime.home.clone(),
        libjvm: i.runtime.libjvm.clone(),
        jvm_args: jvm,
        main_class: i.manifest.main_class.clone(),
        game_args: game,
        working_dir: instance.clone(),
        env: envm,
        log_file: i.dirs.logs().join("game-process.log"),
        width: w, height: h, render_scale: i.profile.render_scale,
        layout_file: i.profile.layout_id.as_ref().map(|s| i.dirs.layouts().join(format!("{s}.json"))).filter(|p| p.is_file()),
        secrets,
    })
}

impl LaunchPlan {
    /// Command line for display/logging with secrets masked.
    pub fn redacted_command(&self) -> String {
        let mut s = format!("{} {}", self.jvm_args.join(" "), self.main_class);
        s.push(' ');
        s.push_str(&self.game_args.join(" "));
        for secret in &self.secrets { s = s.replace(secret.as_str(), "[REDACTED]"); }
        crate::logs::redact(&s)
    }

    /// Writes the plan (it contains the access token) with owner-only permissions. The native
    /// host deletes the file as soon as it has read it.
    pub fn write(&self, dirs: &Dirs) -> Result<PathBuf> {
        let path = dirs.launch_plan();
        write_private(&path, &serde_json::to_vec(self)?)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeInfo;

    fn manifest() -> VersionManifest {
        serde_json::from_str(r#"{
          "id":"26.3","mainClass":"net.minecraft.client.main.Main","type":"release",
          "assetIndex":{"id":"30","sha1":"x","size":1,"url":"u"},
          "arguments":{
            "jvm":["-Djava.library.path=${natives_directory}","-cp","${classpath}"],
            "game":["--username","${auth_player_name}","--accessToken","${auth_access_token}",
                    {"rules":[{"action":"allow","features":{"is_quick_play_multiplayer":true}}],"value":["--quickPlayMultiplayer","${quickPlayMultiplayer}"]}]
          },
          "libraries":[]}"#).unwrap()
    }

    #[test]
    fn builds_plan_with_quickplay_and_backend() {
        let d = tempfile::tempdir().unwrap();
        let dirs = Dirs::new(d.path());
        std::fs::create_dir_all(dirs.version_dir("26.3")).unwrap();
        std::fs::write(dirs.version_dir("26.3").join("26.3.jar"), b"x").unwrap();
        let mut p = Profile::new("T", "26.3");
        p.graphics_backend = crate::profiles::GraphicsBackend::Vulkan;
        let acc = LaunchAccount { username: "Steve".into(), uuid: "u".into(), access_token: "0".into(), microsoft: false };
        let rt = RuntimeInfo { name: "j".into(), home: "/j".into(), libjvm: "/j/lib/server/libjvm.so".into(), java_major: Some(25), os_arch: None };
        let m = manifest();
        let plan = build_plan(&LaunchInputs { dirs: &dirs, profile: &p, manifest: &m, account: &acc, runtime: &rt,
            screen: (2400, 1080), quick_play: Some(("play.example.com".into(), 25565)) }).unwrap();
        assert!(plan.game_args.windows(2).any(|w| w == ["--quickPlayMultiplayer", "play.example.com:25565"]));
        assert!(plan.game_args.windows(2).any(|w| w == ["--graphicsBackend", "vulkan"]));
        assert!(plan.jvm_args.iter().any(|a| a == &format!("-Xmx{}m", p.ram_mb)));
        assert!(plan.jvm_args.iter().any(|a| a.contains("26.3.jar")));
        assert!((1919..=1920).contains(&plan.width)); // 2400 * 0.8
    }
}
