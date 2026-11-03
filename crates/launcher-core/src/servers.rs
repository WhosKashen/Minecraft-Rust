//! Saved servers, validation and Server List Ping status.
//! Offline profiles can only join servers that run with online-mode=false; the launcher never
//! tries to bypass a server's authentication.
use crate::error::{Error, Result};
use crate::paths::Dirs;
use crate::store::{load_json, save_json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub name: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ServerFile { pub servers: Vec<ServerEntry> }

pub fn parse_address(s: &str) -> Result<(String, u16)> {
    let s = s.trim();
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse::<u16>().map_err(|_| Error::invalid("Port must be 1–65535."))?),
        _ => (s, 25565),
    };
    if host.is_empty() || port == 0 || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err(Error::invalid("Enter a server address like play.example.com or play.example.com:25566."));
    }
    Ok((host.to_string(), port))
}

pub fn load(dirs: &Dirs) -> Result<ServerFile> { load_json(&dirs.servers_file()) }
pub fn save(dirs: &Dirs, f: &ServerFile) -> Result<()> { save_json(&dirs.servers_file(), f) }

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub motd: String,
    pub online: u32,
    pub max: u32,
    pub version: String,
    pub latency_ms: u128,
}

pub fn write_varint(out: &mut Vec<u8>, mut v: u32) {
    loop {
        if v & !0x7f == 0 { out.push(v as u8); return; }
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
}

async fn read_varint(s: &mut tokio::net::TcpStream) -> Result<u32> {
    let (mut val, mut shift) = (0u32, 0);
    loop {
        let b = s.read_u8().await?;
        val |= ((b & 0x7f) as u32) << shift;
        if b & 0x80 == 0 { return Ok(val); }
        shift += 7;
        if shift > 35 { return Err(Error::Net("bad VarInt".into())); }
    }
}

fn packet(id: u32, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::new();
    write_varint(&mut inner, id);
    inner.extend_from_slice(body);
    let mut out = Vec::new();
    write_varint(&mut out, inner.len() as u32);
    out.extend(inner);
    out
}

fn motd_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            let mut s = o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if let Some(Value::Array(extra)) = o.get("extra") { for e in extra { s.push_str(&motd_text(e)); } }
            s
        }
        Value::Array(a) => a.iter().map(motd_text).collect(),
        _ => String::new(),
    }
}

pub async fn ping(host: &str, port: u16, protocol: u32) -> Result<ServerStatus> {
    let timeout = std::time::Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        let mut s = tokio::net::TcpStream::connect((host, port)).await?;
        let mut hs = Vec::new();
        write_varint(&mut hs, protocol);
        write_varint(&mut hs, host.len() as u32);
        hs.extend_from_slice(host.as_bytes());
        hs.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut hs, 1);
        s.write_all(&packet(0, &hs)).await?;
        s.write_all(&packet(0, &[])).await?;
        let _len = read_varint(&mut s).await?;
        let id = read_varint(&mut s).await?;
        if id != 0 { return Err(Error::Net("unexpected status packet".into())); }
        let jlen = read_varint(&mut s).await? as usize;
        if jlen > 1 << 20 { return Err(Error::Net("status response too large".into())); }
        let mut buf = vec![0u8; jlen];
        s.read_exact(&mut buf).await?;
        let v: Value = serde_json::from_slice(&buf)?;
        let t0 = std::time::Instant::now();
        s.write_all(&packet(1, &0i64.to_be_bytes())).await?;
        let latency_ms = match read_varint(&mut s).await { Ok(_) => t0.elapsed().as_millis(), Err(_) => 0 };
        Ok(ServerStatus {
            motd: motd_text(&v["description"]),
            online: v["players"]["online"].as_u64().unwrap_or(0) as u32,
            max: v["players"]["max"].as_u64().unwrap_or(0) as u32,
            version: v["version"]["name"].as_str().unwrap_or("?").to_string(),
            latency_ms,
        })
    }).await.map_err(|_| Error::Net("Server did not respond (timed out).".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses() {
        assert_eq!(parse_address("a.b.c").unwrap(), ("a.b.c".into(), 25565));
        assert_eq!(parse_address("a.b:25566").unwrap().1, 25566);
        assert!(parse_address("bad host").is_err());
        assert!(parse_address("x:0").is_err());
    }
    #[test]
    fn varint() {
        let mut v = Vec::new(); write_varint(&mut v, 300);
        assert_eq!(v, vec![0xAC, 0x02]);
    }
    #[test]
    fn motd_extra() {
        let v: Value = serde_json::from_str(r#"{"text":"A","extra":[{"text":"B"},"C"]}"#).unwrap();
        assert_eq!(motd_text(&v), "ABC");
    }
}
