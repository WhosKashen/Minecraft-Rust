//! Inspect a user-supplied Minecraft client JAR: version, Java requirement,
//! graphics/windowing requirements and what must still be provided.
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionJson {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub protocol_version: Option<u32>,
    #[serde(default)]
    pub java_version: Option<u32>,
    #[serde(default)]
    pub java_component: Option<String>,
    #[serde(default)]
    pub build_time: Option<String>,
    #[serde(default)]
    pub stable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarReport {
    pub version: VersionJson,
    pub main_class: Option<String>,
    pub class_count: usize,
    /// Highest class-file major version found (69 = Java 25).
    pub max_class_major: u16,
    /// Java feature version needed to load every class.
    pub required_java: u32,
    pub has_opengl_backend: bool,
    pub has_vulkan_backend: bool,
    pub uses_sdl: bool,
    pub uses_glfw: bool,
    pub signed_by_mojang: bool,
    /// Things the JAR does not contain and the launcher/user must provide.
    pub missing: Vec<String>,
    pub warnings: Vec<String>,
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

pub fn inspect(path: &Path) -> Result<JarReport> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    let mut version: Option<VersionJson> = None;
    let mut main_class = None;
    let mut class_count = 0usize;
    let mut max_major = 0u16;
    let (mut gl, mut vk, mut signed) = (false, false, false);
    let (mut sdl, mut glfw) = (false, false);

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if name == "version.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            version = Some(serde_json::from_str(&s)?);
        } else if name == "META-INF/MANIFEST.MF" {
            // Only the head is needed for Main-Class; the file itself can be megabytes.
            let mut head = Vec::new();
            entry.by_ref().take(4096).read_to_end(&mut head)?;
            let text = String::from_utf8_lossy(&head);
            main_class = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("Main-Class:").map(|v| v.trim().to_string()));
        } else if name.starts_with("META-INF/MOJANGCS.") {
            signed = true;
        } else if name.ends_with(".class") {
            class_count += 1;
            gl |= name.starts_with("com/mojang/renderpearl/backend/opengl/");
            vk |= name.starts_with("com/mojang/renderpearl/backend/vulkan/");
            let mut head = [0u8; 8];
            if entry.read_exact(&mut head).is_ok() && head[..4] == [0xCA, 0xFE, 0xBA, 0xBE] {
                max_major = max_major.max(u16::from_be_bytes([head[6], head[7]]));
            }
            if name == "com/mojang/blaze3d/Blaze3D.class" || name.ends_with("/Window.class") {
                let mut rest = Vec::new();
                entry.read_to_end(&mut rest)?;
                sdl |= contains(&rest, b"org/lwjgl/sdl");
                glfw |= contains(&rest, b"org/lwjgl/glfw");
            }
        }
    }

    let version = version.ok_or_else(|| Error::invalid("version.json not found: this is not a Minecraft client JAR"))?;
    let required_java = if max_major >= 44 { (max_major - 44) as u32 } else { 0 };
    let required_java = version.java_version.map_or(required_java, |v| v.max(required_java));

    let mut missing = vec![
        "Libraries (LWJGL, Netty, authlib, …): downloaded from Mojang's version manifest".to_string(),
        "Asset index and asset objects: downloaded from Mojang".to_string(),
        format!("Java {required_java} runtime for Android arm64 (import a runtime pack in Settings)"),
    ];
    let mut warnings = Vec::new();
    if sdl && !glfw {
        warnings.push("This version uses SDL (not GLFW) for windowing. GLFW-based Android bridges do not work; an SDL-capable LWJGL build is required.".into());
        missing.push("Android-compatible LWJGL build with SDL support".into());
    }
    if gl && !vk {
        warnings.push("Only an OpenGL backend was found.".into());
    }
    if gl {
        warnings.push("The OpenGL backend needs desktop OpenGL 3.3 core plus several ARB extensions; Android only provides OpenGL ES, so a translation layer is needed. The Vulkan backend avoids that but needs a capable GPU driver.".into());
    }
    if !signed {
        warnings.push("JAR is not Mojang-signed; it may have been modified.".into());
    }
    if main_class.is_none() {
        warnings.push("No Main-Class in manifest; defaulting to net.minecraft.client.main.Main.".into());
    }

    Ok(JarReport {
        version, main_class, class_count, max_class_major: max_major, required_java,
        has_opengl_backend: gl, has_vulkan_backend: vk, uses_sdl: sdl, uses_glfw: glfw,
        signed_by_mojang: signed, missing, warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn inspects_synthetic_jar() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.jar");
        let f = std::fs::File::create(&p).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("version.json", o).unwrap();
        z.write_all(br#"{"id":"26.3","java_version":25,"protocol_version":777}"#).unwrap();
        z.start_file("a/B.class", o).unwrap();
        z.write_all(&[0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 69]).unwrap();
        z.finish().unwrap();
        let r = inspect(&p).unwrap();
        assert_eq!(r.version.id, "26.3");
        assert_eq!(r.max_class_major, 69);
        assert_eq!(r.required_java, 25);
        assert!(!r.signed_by_mojang);
    }
}
