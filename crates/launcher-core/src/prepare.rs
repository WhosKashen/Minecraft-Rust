//! Game preparation: inspect the supplied JAR, fetch the matching Mojang version JSON,
//! libraries and assets, and install the user's own client JAR. Mojang's client JAR is never
//! downloaded; the supplied file is used as-is.
use crate::assets::{asset_tasks, AssetIndex};
use crate::download::{download_all, download_one, get_json_text, http_client, sha1_file, DownloadTask, Progress};
use crate::error::{Error, Result};
use crate::jar_inspect::{self, JarReport};
use crate::logs;
use crate::manifest::*;
use crate::paths::Dirs;
use std::path::Path;
use std::sync::Arc;

pub struct Prepared {
    pub report: JarReport,
    pub manifest: VersionManifest,
    pub warnings: Vec<String>,
}

pub fn library_env() -> RuleEnv { RuleEnv::android_arm64(&[]) }

pub fn library_tasks(m: &VersionManifest, dirs: &Dirs) -> Vec<DownloadTask> {
    let env = library_env();
    m.libraries.iter()
        .filter(|l| rules_allow(&l.rules, &env))
        .filter_map(|l| l.downloads.as_ref()?.artifact.as_ref())
        .map(|a| DownloadTask {
            url: a.url.clone(),
            dest: dirs.libraries().join(&a.path),
            sha1: Some(a.sha1.clone()),
            size: Some(a.size),
        })
        .collect()
}

async fn load_version_json(dirs: &Dirs, id: &str, client: &reqwest::Client, p: &Progress) -> Result<VersionManifest> {
    let cached = dirs.version_dir(id).join(format!("{id}.json"));
    if let Ok(s) = std::fs::read_to_string(&cached) {
        if let Ok(m) = serde_json::from_str::<VersionManifest>(&s) { return Ok(m); }
    }
    p.set_label("Fetching version manifest");
    let idx: VersionManifestIndex = serde_json::from_str(&get_json_text(client, VERSION_MANIFEST_URL).await?)?;
    let entry = idx.versions.iter().find(|v| v.id == id).ok_or_else(|| Error::invalid(format!(
        "Minecraft version '{id}' was not found in Mojang's version manifest, so its libraries cannot be resolved.")))?;
    let text = get_json_text(client, &entry.url).await?;
    if let Some(want) = &entry.sha1 {
        use sha1::{Digest, Sha1};
        let got = hex::encode(Sha1::digest(text.as_bytes()));
        if !got.eq_ignore_ascii_case(want) {
            return Err(Error::Checksum { file: format!("{id}.json"), expected: want.clone(), actual: got });
        }
    }
    let m: VersionManifest = serde_json::from_str(&text)?;
    std::fs::create_dir_all(dirs.version_dir(id))?;
    std::fs::write(&cached, text)?;
    Ok(m)
}

/// Full preparation. Progress is observable through `p`; cancel via `p.cancel()`.
pub async fn prepare(dirs: &Dirs, jar: &Path, p: Arc<Progress>) -> Result<Prepared> {
    dirs.ensure_all()?;
    p.set_label("Inspecting client JAR");
    let report = jar_inspect::inspect(jar)?;
    let id = report.version.id.clone();
    let mut warnings = report.warnings.clone();
    let client = http_client()?;

    let manifest = load_version_json(dirs, &id, &client, &p).await?;

    // Install the user's JAR (copy only when missing or different).
    let dest = dirs.version_dir(&id).join(format!("{id}.jar"));
    let jar_hash = sha1_file(jar)?;
    if !dest.exists() || sha1_file(&dest)? != jar_hash {
        p.set_label("Installing client JAR");
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::copy(jar, &dest)?;
    }
    if let Some(official) = manifest.downloads.get("client") {
        if !official.sha1.eq_ignore_ascii_case(&jar_hash) {
            warnings.push("The supplied client JAR differs from Mojang's official one (different checksum). It is used as supplied.".into());
        }
    }

    p.set_label("Downloading libraries");
    let libs = library_tasks(&manifest, dirs);
    logs::info(&format!("{} libraries to verify for {id}", libs.len()));
    download_all(&client, libs, 6, p.clone()).await?;

    p.set_label("Downloading asset index");
    let idx_path = dirs.assets().join("indexes").join(format!("{}.json", manifest.asset_index.id));
    download_one(&client, &DownloadTask {
        url: manifest.asset_index.url.clone(), dest: idx_path.clone(),
        sha1: Some(manifest.asset_index.sha1.clone()), size: Some(manifest.asset_index.size),
    }, &p).await?;
    let index: AssetIndex = serde_json::from_str(&std::fs::read_to_string(&idx_path)?)?;

    p.set_label("Downloading game assets");
    download_all(&client, asset_tasks(&index, dirs), 8, p.clone()).await?;
    p.set_label("Ready");
    Ok(Prepared { report, manifest, warnings })
}
