//! Per-profile mod management: scan, metadata, enable/disable, import, dependency checks.
use crate::error::{Error, Result};
use crate::paths::safe_file_name;
use crate::profiles::{Issue, Loader};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ModInfo {
    pub file: PathBuf,
    pub enabled: bool,
    pub loader: Option<Loader>,
    pub id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    /// (mod id, version predicate or empty) for hard dependencies.
    pub depends: Vec<(String, String)>,
    pub has_native_libs: bool,
}

const BUILTIN_IDS: &[&str] = &["minecraft", "java", "fabricloader", "fabric-loader", "quilt_loader", "forge", "neoforge", "fml", "javafml", "mixinextras"];

fn read_entry(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut e = z.by_name(name).ok()?;
    let mut s = String::new();
    e.by_ref().take(1 << 20).read_to_string(&mut s).ok()?;
    Some(s)
}

fn pred_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" || "),
        _ => String::new(),
    }
}

pub fn read_info(path: &Path) -> ModInfo {
    let disabled = path.extension().map_or(false, |e| e == "disabled");
    let mut info = ModInfo {
        file: path.into(), enabled: !disabled, loader: None, id: None,
        name: path.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(".disabled").to_string(),
        version: None, depends: vec![], has_native_libs: false,
    };
    let Ok(f) = std::fs::File::open(path) else { return info };
    let Ok(mut z) = zip::ZipArchive::new(f) else { return info };
    info.has_native_libs = z.file_names().any(|n| {
        let l = n.to_lowercase();
        l.ends_with(".so") || l.ends_with(".dll") || l.ends_with(".dylib") || l.ends_with(".jnilib")
    });
    if let Some(t) = read_entry(&mut z, "fabric.mod.json") {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            info.loader = Some(Loader::Fabric);
            info.id = v["id"].as_str().map(str::to_string);
            info.version = v["version"].as_str().map(str::to_string);
            if let Some(n) = v["name"].as_str() { info.name = n.into(); }
            if let Some(d) = v["depends"].as_object() {
                info.depends = d.iter().map(|(k, p)| (k.clone(), pred_to_string(p))).collect();
            }
        }
    } else if let Some(t) = read_entry(&mut z, "quilt.mod.json") {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            info.loader = Some(Loader::Quilt);
            let q = &v["quilt_loader"];
            info.id = q["id"].as_str().map(str::to_string);
            info.version = q["version"].as_str().map(str::to_string);
            if let Some(n) = v["metadata"]["name"].as_str() { info.name = n.into(); }
            if let Some(a) = q["depends"].as_array() {
                for d in a {
                    match d {
                        Value::String(s) => info.depends.push((s.clone(), String::new())),
                        Value::Object(o) => if let Some(id) = o.get("id").and_then(|x| x.as_str()) {
                            if o.get("optional").and_then(|x| x.as_bool()) != Some(true) {
                                info.depends.push((id.into(), o.get("versions").map(pred_to_string).unwrap_or_default()));
                            }
                        },
                        _ => {}
                    }
                }
            }
        }
    } else {
        let (toml_name, loader) = if z.by_name("META-INF/neoforge.mods.toml").is_ok() {
            ("META-INF/neoforge.mods.toml", Loader::NeoForge)
        } else { ("META-INF/mods.toml", Loader::Forge) };
        if let Some(t) = read_entry(&mut z, toml_name) {
            if let Ok(v) = t.parse::<toml::Table>() {
                info.loader = Some(loader);
                if let Some(m) = v.get("mods").and_then(|m| m.as_array()).and_then(|a| a.first()) {
                    info.id = m.get("modId").and_then(|x| x.as_str()).map(str::to_string);
                    info.version = m.get("version").and_then(|x| x.as_str()).map(str::to_string);
                    if let Some(n) = m.get("displayName").and_then(|x| x.as_str()) { info.name = n.into(); }
                }
                if let (Some(id), Some(deps)) = (&info.id, v.get("dependencies").and_then(|d| d.as_table())) {
                    if let Some(list) = deps.get(id).and_then(|l| l.as_array()) {
                        for d in list {
                            let mandatory = d.get("mandatory").and_then(|x| x.as_bool())
                                .or_else(|| d.get("type").and_then(|x| x.as_str()).map(|t| t == "required")).unwrap_or(false);
                            if mandatory {
                                if let Some(did) = d.get("modId").and_then(|x| x.as_str()) {
                                    info.depends.push((did.into(), d.get("versionRange").and_then(|x| x.as_str()).unwrap_or("").into()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    info
}

pub fn scan(mods_dir: &Path) -> Vec<ModInfo> {
    let mut v: Vec<ModInfo> = std::fs::read_dir(mods_dir).map(|rd| {
        rd.flatten().map(|e| e.path()).filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            n.ends_with(".jar") || n.ends_with(".jar.disabled")
        }).map(|p| read_info(&p)).collect()
    }).unwrap_or_default();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

pub fn import(mods_dir: &Path, src: &Path, original_name: &str) -> Result<PathBuf> {
    let name = safe_file_name(original_name).ok_or_else(|| Error::invalid("Invalid file name."))?;
    if !name.to_lowercase().ends_with(".jar") { return Err(Error::invalid("Only .jar mod files can be imported.")); }
    zip::ZipArchive::new(std::fs::File::open(src)?).map_err(|_| Error::invalid("This file is not a valid JAR/ZIP."))?;
    std::fs::create_dir_all(mods_dir)?;
    let dest = mods_dir.join(name);
    std::fs::copy(src, &dest)?;
    Ok(dest)
}

pub fn set_enabled(mod_path: &Path, enable: bool) -> Result<PathBuf> {
    let s = mod_path.to_string_lossy().to_string();
    let new = if enable { s.strip_suffix(".disabled").map(str::to_string) } else if s.ends_with(".jar") { Some(format!("{s}.disabled")) } else { None };
    match new {
        Some(n) => { std::fs::rename(mod_path, &n)?; Ok(PathBuf::from(n)) }
        None => Ok(mod_path.into()),
    }
}

pub fn remove(mod_path: &Path) -> Result<()> { Ok(std::fs::remove_file(mod_path)?) }

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<u64> = a.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(0)).collect();
    let pb: Vec<u64> = b.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(0)).collect();
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i).copied().unwrap_or(0), pb.get(i).copied().unwrap_or(0));
        if x != y { return x.cmp(&y); }
    }
    std::cmp::Ordering::Equal
}

/// Conservative predicate check: unknown syntax counts as satisfied so we never report false errors.
pub fn version_satisfies(pred: &str, ver: &str) -> bool {
    let pred = pred.trim();
    if pred.is_empty() || pred == "*" { return true; }
    pred.split("||").any(|alt| {
        alt.split_whitespace().all(|c| {
            use std::cmp::Ordering::*;
            if c == "*" { return true; }
            if let Some(r) = c.strip_prefix(">=") { return cmp_ver(ver, r) != Less; }
            if let Some(r) = c.strip_prefix("<=") { return cmp_ver(ver, r) != Greater; }
            if let Some(r) = c.strip_prefix('>') { return cmp_ver(ver, r) == Greater; }
            if let Some(r) = c.strip_prefix('<') { return cmp_ver(ver, r) == Less; }
            if let Some(r) = c.strip_prefix('=') { return cmp_ver(ver, r) == Equal; }
            if let Some(r) = c.strip_prefix('~').or_else(|| c.strip_prefix('^')) {
                let prefix: Vec<&str> = r.split('.').take(2).collect();
                return ver.split('.').take(prefix.len()).eq(prefix.iter().copied());
            }
            if c.contains('x') || c.contains('X') || c.contains('*') {
                let parts: Vec<&str> = c.split('.').collect();
                return parts.iter().zip(ver.split('.')).all(|(p, v)| matches!(*p, "x" | "X" | "*") || *p == v);
            }
            if c.starts_with(|ch: char| ch.is_ascii_digit()) { return cmp_ver(ver, c) == Equal; }
            true
        })
    })
}

/// Checks loader compatibility, duplicates, missing hard dependencies and Minecraft version.
pub fn analyze(mods: &[ModInfo], loader: Loader, mc_version: &str) -> Vec<Issue> {
    let mut v = Vec::new();
    let enabled: Vec<&ModInfo> = mods.iter().filter(|m| m.enabled).collect();
    let mut ids: HashMap<&str, &ModInfo> = HashMap::new();
    let issue = |error: bool, m: String| Issue { error, message: m };
    for m in &enabled {
        if let Some(l) = m.loader {
            let ok = l == loader || (loader == Loader::Quilt && l == Loader::Fabric);
            if !ok {
                v.push(issue(true, format!("{} is a {:?} mod but this profile uses {:?}. Forge, NeoForge and Fabric mods are not interchangeable.", m.name, l, loader)));
            }
        } else if loader != Loader::Vanilla {
            v.push(issue(false, format!("{}: could not read mod metadata.", m.name)));
        }
        if m.has_native_libs {
            v.push(issue(false, format!("{} bundles native libraries (.so/.dll/.dylib). Desktop natives cannot load on Android; the mod may crash or disable features.", m.name)));
        }
        if let Some(id) = &m.id {
            if let Some(prev) = ids.insert(id.as_str(), m) {
                v.push(issue(true, format!("Duplicate mod id '{id}': {} and {}.", prev.name, m.name)));
            }
        }
    }
    for m in &enabled {
        for (dep, pred) in &m.depends {
            if dep == "minecraft" {
                if !version_satisfies(pred, mc_version) {
                    v.push(issue(true, format!("{} requires Minecraft {pred}, but this profile is {mc_version}.", m.name)));
                }
            } else if BUILTIN_IDS.contains(&dep.as_str()) {
                continue;
            } else {
                match ids.get(dep.as_str()) {
                    None => v.push(issue(true, format!("{} needs '{dep}' which is not installed or is disabled.", m.name))),
                    Some(d) => if let Some(ver) = &d.version {
                        if !version_satisfies(pred, ver) {
                            v.push(issue(true, format!("{} needs {dep} {pred}, but {ver} is installed.", m.name)));
                        }
                    },
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(dir: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let p = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        for (n, c) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(c.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        p
    }

    #[test]
    fn predicates() {
        assert!(version_satisfies(">=1.21 <1.22", "1.21.4"));
        assert!(!version_satisfies(">=1.21 <1.22", "1.22"));
        assert!(version_satisfies("1.21.x", "1.21.9"));
        assert!(version_satisfies("~26.3", "26.3.1"));
        assert!(version_satisfies("1.20 || 26.3", "26.3"));
    }

    #[test]
    fn detects_missing_dep_and_loader_mismatch() {
        let d = tempfile::tempdir().unwrap();
        jar(d.path(), "a.jar", &[("fabric.mod.json", r#"{"id":"a","version":"1","depends":{"b":"*","minecraft":">=26"}}"#)]);
        jar(d.path(), "f.jar", &[("META-INF/mods.toml", "[[mods]]\nmodId=\"f\"\nversion=\"1\"\n")]);
        let mods = scan(d.path());
        let issues = analyze(&mods, Loader::Fabric, "26.3");
        assert!(issues.iter().any(|i| i.message.contains("needs 'b'")));
        assert!(issues.iter().any(|i| i.message.contains("not interchangeable")));
    }

    #[test]
    fn enable_disable_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = jar(d.path(), "x.jar", &[("a", "b")]);
        let off = set_enabled(&p, false).unwrap();
        assert!(off.to_string_lossy().ends_with(".jar.disabled"));
        let on = set_enabled(&off, true).unwrap();
        assert_eq!(on, p);
    }
}
