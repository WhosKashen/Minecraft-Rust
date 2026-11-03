//! Java runtime registry and import. Runtimes must be built for Android (bionic) arm64;
//! desktop JDKs will not load.
use crate::error::{Error, Result};
use crate::paths::Dirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub name: String,
    pub home: PathBuf,
    pub libjvm: PathBuf,
    pub java_major: Option<u32>,
    pub os_arch: Option<String>,
}

fn find_libjvm(home: &Path) -> Option<PathBuf> {
    ["lib/server/libjvm.so", "lib/aarch64/server/libjvm.so", "jre/lib/aarch64/server/libjvm.so"]
        .iter()
        .map(|r| home.join(r))
        .find(|p| p.is_file())
}

fn release_value(home: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(home.join("release")).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

pub fn parse_java_major(version: &str) -> Option<u32> {
    let first = version.split(['.', '_', '-', '+']).next()?;
    let n: u32 = first.parse().ok()?;
    if n == 1 { version.split('.').nth(1)?.parse().ok() } else { Some(n) }
}

pub fn describe(name: &str, home: &Path) -> Option<RuntimeInfo> {
    let libjvm = find_libjvm(home)?;
    Some(RuntimeInfo {
        name: name.into(),
        home: home.into(),
        libjvm,
        java_major: release_value(home, "JAVA_VERSION").and_then(|v| parse_java_major(&v)),
        os_arch: release_value(home, "OS_ARCH"),
    })
}

pub fn scan(dirs: &Dirs) -> Vec<RuntimeInfo> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dirs.runtimes()) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                if let Some(r) = describe(&e.file_name().to_string_lossy(), &e.path()) { v.push(r); }
            }
        }
    }
    v.sort_by(|a, b| b.java_major.cmp(&a.java_major));
    v
}

/// Pick the named runtime, or the lowest installed runtime that satisfies `required_major`.
pub fn select(dirs: &Dirs, preferred: Option<&str>, required_major: u32) -> Result<RuntimeInfo> {
    let all = scan(dirs);
    if let Some(n) = preferred {
        let r = all.iter().find(|r| r.name == n).cloned()
            .ok_or_else(|| Error::invalid(format!("Runtime '{n}' is not installed.")))?;
        if r.java_major.map_or(false, |m| m < required_major) {
            return Err(Error::invalid(format!("Runtime '{n}' is Java {} but this version needs Java {required_major} or newer.", r.java_major.unwrap())));
        }
        return Ok(r);
    }
    all.into_iter()
        .filter(|r| r.java_major.map_or(false, |m| m >= required_major))
        .min_by_key(|r| r.java_major)
        .ok_or_else(|| Error::invalid(format!("No Java {required_major}+ runtime for Android arm64 is installed. Import a runtime pack in Settings → Runtimes.")))
}

fn find_runtime_root(base: &Path, depth: u32) -> Option<PathBuf> {
    if find_libjvm(base).is_some() { return Some(base.into()); }
    if depth == 0 { return None; }
    for e in std::fs::read_dir(base).ok()?.flatten() {
        if e.path().is_dir() {
            if let Some(r) = find_runtime_root(&e.path(), depth - 1) { return Some(r); }
        }
    }
    None
}

/// Import a .zip or .tar.gz/.tgz runtime archive into runtimes/<name>.
pub fn import_archive(dirs: &Dirs, archive: &Path, name: &str) -> Result<RuntimeInfo> {
    let name = crate::paths::safe_file_name(name).ok_or_else(|| Error::invalid("Invalid runtime name."))?;
    let tmp = dirs.runtimes().join(format!("{name}.importing"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let lower = archive.to_string_lossy().to_lowercase();
    let res: Result<()> = (|| {
        if lower.ends_with(".zip") {
            let mut z = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
            for i in 0..z.len() {
                let mut e = z.by_index(i)?;
                let Some(rel) = e.enclosed_name() else { continue }; // skips traversal paths
                let out = tmp.join(rel);
                if e.is_dir() { std::fs::create_dir_all(&out)?; continue; }
                if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
                let mut f = std::fs::File::create(&out)?;
                std::io::copy(&mut e, &mut f)?;
                #[cfg(unix)]
                if let Some(mode) = e.unix_mode() {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
                }
            }
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            let gz = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
            tar::Archive::new(gz).unpack(&tmp)?; // tar rejects paths escaping the destination
        } else {
            return Err(Error::invalid("Unsupported archive type. Use .zip or .tar.gz."));
        }
        Ok(())
    })();
    if let Err(e) = res { let _ = std::fs::remove_dir_all(&tmp); return Err(e); }
    let root = find_runtime_root(&tmp, 3).ok_or_else(|| {
        let _ = std::fs::remove_dir_all(&tmp);
        Error::invalid("No libjvm.so found in the archive. It must be a Java runtime built for Android arm64.")
    })?;
    let dest = dirs.runtimes().join(name);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&root, &dest)?;
    let _ = std::fs::remove_dir_all(&tmp);
    describe(name, &dest).ok_or_else(|| Error::invalid("Runtime import failed."))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_versions() {
        assert_eq!(parse_java_major("25.0.1"), Some(25));
        assert_eq!(parse_java_major("1.8.0_402"), Some(8));
        assert_eq!(parse_java_major("21"), Some(21));
    }
}
