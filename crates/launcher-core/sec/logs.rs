//! Launcher log buffer, secret redaction and log/crash export.
use crate::error::Result;
use crate::paths::Dirs;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const MAX_LINES: usize = 2000;

#[derive(Default)]
struct State {
    lines: VecDeque<String>,
    secrets: Vec<String>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}

/// Register a secret value (token, etc.) so it is masked in every later log line and export.
pub fn register_secret(secret: &str) {
    if secret.len() >= 6 {
        let mut s = state().lock().unwrap();
        if !s.secrets.iter().any(|x| x == secret) { s.secrets.push(secret.to_string()); }
    }
}

/// Mask registered secrets and anything that looks like a JWT / bearer token.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for s in state().lock().unwrap().secrets.iter() {
        out = out.replace(s.as_str(), "[REDACTED]");
    }
    mask_jwt_like(&out)
}

fn mask_jwt_like(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if s[i..].starts_with("eyJ") {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'-' | b'_' | b'.')) { j += 1; }
            if j - i > 40 { out.push_str("[REDACTED]"); i = j; continue; }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn log(level: &str, msg: &str) {
    let line = redact(&format!("[{level}] {msg}"));
    let mut s = state().lock().unwrap();
    if s.lines.len() >= MAX_LINES { s.lines.pop_front(); }
    s.lines.push_back(line);
}
pub fn info(msg: &str) { log("INFO", msg) }
pub fn warn(msg: &str) { log("WARN", msg) }
pub fn error(msg: &str) { log("ERROR", msg) }

pub fn snapshot() -> Vec<String> { state().lock().unwrap().lines.iter().cloned().collect() }
pub fn clear() { state().lock().unwrap().lines.clear(); }

fn read_text_limited(path: &Path, max: u64) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len > max {
        use std::io::{Seek, SeekFrom};
        f.seek(SeekFrom::Start(len - max))?; // keep the tail, where the useful part is
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Bundle launcher log + game latest.log + crash reports + JVM crash files into one ZIP (all redacted).
pub fn export_bundle(dirs: &Dirs, instance_dir: Option<&Path>, out: &Path) -> Result<()> {
    let f = std::fs::File::create(out)?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default();
    z.start_file("launcher.log", opts)?;
    z.write_all(snapshot().join("\n").as_bytes())?;
    if let Ok(rd) = std::fs::read_dir(dirs.logs()) {
        for e in rd.flatten() {
            if let Ok(t) = read_text_limited(&e.path(), 2 * 1024 * 1024) {
                z.start_file(format!("logs/{}", e.file_name().to_string_lossy()), opts)?;
                z.write_all(redact(&t).as_bytes())?;
            }
        }
    }
    if let Some(inst) = instance_dir {
        let mut add = |rel: &str, name: String| -> Result<()> {
            if let Ok(t) = read_text_limited(&inst.join(rel), 2 * 1024 * 1024) {
                z.start_file(name, opts)?;
                z.write_all(redact(&t).as_bytes())?;
            }
            Ok(())
        };
        add("logs/latest.log", "game/latest.log".into())?;
        for sub in ["crash-reports"] {
            if let Ok(rd) = std::fs::read_dir(inst.join(sub)) {
                for e in rd.flatten().take(10) {
                    let rel = format!("{sub}/{}", e.file_name().to_string_lossy());
                    add(&rel, format!("game/{rel}"))?;
                }
            }
        }
        if let Ok(rd) = std::fs::read_dir(inst) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with("hs_err_pid") { add(&n, format!("game/{n}"))?; }
            }
        }
    }
    z.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks_registered_and_jwt() {
        register_secret("supersecretvalue");
        assert_eq!(redact("x supersecretvalue y"), "x [REDACTED] y");
        let jwt = format!("eyJ{}", "a".repeat(60));
        assert_eq!(redact(&format!("Bearer {jwt}")), "Bearer [REDACTED]");
        assert_eq!(redact("héllo wörld"), "héllo wörld");
    }
}
