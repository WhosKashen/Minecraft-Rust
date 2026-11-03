//! Mojang version-manifest and per-version JSON models, plus rule evaluation.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const VERSION_MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const ASSET_BASE_URL: &str = "https://resources.download.minecraft.net";

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifestIndex {
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionEntry {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionManifest {
    pub id: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    /// Legacy (pre-1.13) single-string argument form.
    #[serde(rename = "minecraftArguments", default)]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "assetIndex")]
    pub asset_index: AssetIndexRef,
    #[serde(default)]
    pub assets: String,
    #[serde(default)]
    pub downloads: HashMap<String, DownloadInfo>,
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersionReq>,
    #[serde(default)]
    pub libraries: Vec<Library>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Ruled { rules: Vec<Rule>, value: StringOrVec },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

impl StringOrVec {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrVec::One(s) => vec![s.clone()],
            StringOrVec::Many(v) => v.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JavaVersionReq {
    pub component: String,
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadInfo {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Artifact {
    pub path: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

/// Environment used to evaluate rules. Android is reported as Linux/arm64.
#[derive(Debug, Clone)]
pub struct RuleEnv {
    pub os_name: String,
    pub arch: String,
    pub features: HashMap<String, bool>,
}

impl RuleEnv {
    pub fn android_arm64(features: &[(&str, bool)]) -> Self {
        Self {
            os_name: "linux".into(),
            arch: "arm64".into(),
            features: features.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }
}

fn rule_matches(rule: &Rule, env: &RuleEnv) -> bool {
    if let Some(os) = &rule.os {
        if let Some(n) = &os.name {
            if n != &env.os_name { return false; }
        }
        if let Some(a) = &os.arch {
            if a != &env.arch { return false; }
        }
    }
    if let Some(f) = &rule.features {
        for (k, v) in f {
            if env.features.get(k).copied().unwrap_or(false) != *v { return false; }
        }
    }
    true
}

/// Mojang semantics: no rules => allowed; otherwise the last matching rule decides, default deny.
pub fn rules_allow(rules: &[Rule], env: &RuleEnv) -> bool {
    if rules.is_empty() { return true; }
    let mut allowed = false;
    for r in rules {
        if rule_matches(r, env) {
            allowed = r.action == "allow";
        }
    }
    allowed
}

pub fn expand_arguments(args: &[Argument], env: &RuleEnv) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Argument::Plain(s) => out.push(s.clone()),
            Argument::Ruled { rules, value } => {
                if rules_allow(rules, env) { out.extend(value.to_vec()); }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_last_match_wins() {
        let env = RuleEnv::android_arm64(&[]);
        let r: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#).unwrap();
        assert!(rules_allow(&r, &env));
        let r2: Vec<Rule> = serde_json::from_str(r#"[{"action":"allow","os":{"name":"windows"}}]"#).unwrap();
        assert!(!rules_allow(&r2, &env));
    }

    #[test]
    fn parses_mixed_arguments() {
        let a: Arguments = serde_json::from_str(r#"{"game":["--username","${auth_player_name}",
          {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}"]}],
          "jvm":[{"rules":[{"action":"allow","os":{"name":"osx"}}],"value":"-XstartOnFirstThread"},"-cp","${classpath}"]}"#).unwrap();
        let env = RuleEnv::android_arm64(&[("has_custom_resolution", true)]);
        assert_eq!(expand_arguments(&a.game, &env).len(), 4);
        assert_eq!(expand_arguments(&a.jvm, &env), vec!["-cp", "${classpath}"]);
    }
}
