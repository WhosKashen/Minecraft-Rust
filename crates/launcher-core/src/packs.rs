//! Resource packs, shader packs and in-game selection/order (options.txt, Iris config).
use crate::error::{Error, Result};
use crate::paths::safe_file_name;
use std::path::{Path, PathBuf};

fn copy_in(dir: &Path, src: &Path, original_name: &str) -> Result<PathBuf> {
    let name = safe_file_name(original_name).ok_or_else(|| Error::invalid("Invalid file name."))?;
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(name);
    std::fs::copy(src, &dest)?;
    Ok(dest)
}

fn zip_has(src: &Path, f: impl Fn(&str) -> bool) -> Result<bool> {
    let z = zip::ZipArchive::new(std::fs::File::open(src)?).map_err(|_| Error::invalid("This file is not a valid ZIP."))?;
    Ok(z.file_names().any(|n| f(n)))
}

pub fn import_resource_pack(instance: &Path, src: &Path, name: &str) -> Result<PathBuf> {
    if !zip_has(src, |n| n == "pack.mcmeta")? {
        return Err(Error::invalid("Not a resource pack: pack.mcmeta was not found at the top level of the ZIP (the pack may be wrapped in an extra folder)."));
    }
    copy_in(&instance.join("resourcepacks"), src, name)
}

pub fn import_shader_pack(instance: &Path, src: &Path, name: &str) -> Result<PathBuf> {
    if !zip_has(src, |n| n.starts_with("shaders/"))? {
        return Err(Error::invalid("Not a shader pack: no top-level 'shaders/' folder in the ZIP (it may be wrapped in an extra folder)."));
    }
    copy_in(&instance.join("shaderpacks"), src, name)
}

pub fn list_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

pub fn remove_file(dir: &Path, name: &str) -> Result<()> {
    let n = safe_file_name(name).ok_or_else(|| Error::invalid("Invalid file name."))?;
    let p = dir.join(n);
    if p.is_dir() { std::fs::remove_dir_all(p)?; } else { std::fs::remove_file(p)?; }
    Ok(())
}

// ---------------------------------------------------------------- options.txt

const RP_KEY: &str = "resourcePacks:";

/// Enabled resource packs in the game's own order (first = lowest priority). Entries look like
/// "vanilla", "file/MyPack.zip", "fabric".
pub fn enabled_resource_packs(instance: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(instance.join("options.txt")).unwrap_or_default();
    text.lines().find_map(|l| l.strip_prefix(RP_KEY))
        .and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
        .unwrap_or_else(|| vec!["vanilla".into()])
}

pub fn set_enabled_resource_packs(instance: &Path, packs: &[String]) -> Result<()> {
    let path = instance.join("options.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let line = format!("{RP_KEY}{}", serde_json::to_string(packs)?);
    let mut found = false;
    let mut lines: Vec<String> = text.lines().map(|l| if l.starts_with(RP_KEY) { found = true; line.clone() } else { l.to_string() }).collect();
    if !found { lines.push(line); }
    std::fs::create_dir_all(instance)?;
    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

pub fn pack_entry_for_file(file_name: &str) -> String { format!("file/{file_name}") }

/// Select (or clear) the Iris shader pack via config/iris.properties, preserving other keys.
pub fn set_iris_shader(instance: &Path, pack: Option<&str>) -> Result<()> {
    let path = instance.join("config").join("iris.properties");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with("shaderPack=") && !l.starts_with("enableShaders=")).map(str::to_string).collect();
    lines.push(format!("shaderPack={}", pack.unwrap_or("")));
    lines.push(format!("enableShaders={}", pack.is_some()));
    if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn order_roundtrip_preserves_other_options() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("options.txt"), "fov:70.0\nresourcePacks:[\"vanilla\"]\nlang:en_us\n").unwrap();
        set_enabled_resource_packs(d.path(), &["vanilla".into(), "file/a.zip".into(), "file/b.zip".into()]).unwrap();
        assert_eq!(enabled_resource_packs(d.path()), vec!["vanilla", "file/a.zip", "file/b.zip"]);
        let t = std::fs::read_to_string(d.path().join("options.txt")).unwrap();
        assert!(t.contains("fov:70.0") && t.contains("lang:en_us"));
    }
    #[test]
    fn rejects_non_pack_zip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("wrapped/pack.mcmeta", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"{}").unwrap(); z.finish().unwrap();
        assert!(import_resource_pack(d.path(), &p, "x.zip").is_err());
    }
}
