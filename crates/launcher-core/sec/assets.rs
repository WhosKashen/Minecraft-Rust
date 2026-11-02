use crate::download::DownloadTask;
use crate::manifest::ASSET_BASE_URL;
use crate::paths::Dirs;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct AssetIndex {
    pub objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

pub fn asset_tasks(index: &AssetIndex, dirs: &Dirs) -> Vec<DownloadTask> {
    let mut seen = std::collections::HashSet::new();
    let mut v = Vec::new();
    for o in index.objects.values() {
        if o.hash.len() < 2 || !seen.insert(o.hash.clone()) { continue; }
        let prefix = &o.hash[..2];
        v.push(DownloadTask {
            url: format!("{ASSET_BASE_URL}/{prefix}/{}", o.hash),
            dest: dirs.assets().join("objects").join(prefix).join(&o.hash),
            sha1: Some(o.hash.clone()),
            size: Some(o.size),
        });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedups_and_prefixes() {
        let idx: AssetIndex = serde_json::from_str(
            r#"{"objects":{"a":{"hash":"abcdef","size":1},"b":{"hash":"abcdef","size":1}}}"#).unwrap();
        let t = asset_tasks(&idx, &Dirs::new("/x"));
        assert_eq!(t.len(), 1);
        assert!(t[0].url.ends_with("/ab/abcdef"));
    }
}
