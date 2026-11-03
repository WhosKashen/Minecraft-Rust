//! Accounts: clearly separated offline profiles and Microsoft/Minecraft accounts.
//! Tokens are never written to accounts.json; they go through a platform `SecretStore`
//! (Android Keystore-backed on device) and are registered with the log redactor.
use crate::error::{Error, Result};
use crate::logs;
use crate::paths::Dirs;
use crate::store::{load_json, save_json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Offline,
    Microsoft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub username: String,
    /// Dashed UUID. Offline: derived from the name. Microsoft: Mojang profile id.
    pub uuid: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AccountFile {
    pub selected: Option<String>,
    pub accounts: Vec<Account>,
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// In-memory store for tests / desktop dev builds.
#[derive(Default)]
pub struct MemorySecretStore(std::sync::Mutex<std::collections::HashMap<String, String>>);
impl SecretStore for MemorySecretStore {
    fn get(&self, k: &str) -> Option<String> { self.0.lock().unwrap().get(k).cloned() }
    fn set(&self, k: &str, v: &str) -> Result<()> { self.0.lock().unwrap().insert(k.into(), v.into()); Ok(()) }
    fn delete(&self, k: &str) -> Result<()> { self.0.lock().unwrap().remove(k); Ok(()) }
}

pub fn validate_offline_name(name: &str) -> Result<()> {
    let ok = (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok { Ok(()) } else { Err(Error::invalid("Offline names must be 3–16 characters: letters, digits or underscore.")) }
}

/// Same derivation vanilla servers use for offline-mode players: UUIDv3 of "OfflinePlayer:<name>".
pub fn offline_uuid(name: &str) -> String {
    let mut d = md5::compute(format!("OfflinePlayer:{name}").as_bytes()).0;
    d[6] = (d[6] & 0x0f) | 0x30;
    d[8] = (d[8] & 0x3f) | 0x80;
    let h = hex::encode(d);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn dash_uuid(raw: &str) -> String {
    let r: String = raw.chars().filter(|c| *c != '-').collect();
    if r.len() != 32 { return raw.to_string(); }
    format!("{}-{}-{}-{}-{}", &r[0..8], &r[8..12], &r[12..16], &r[16..20], &r[20..32])
}

pub struct AccountStore {
    pub file: AccountFile,
    dirs: Dirs,
}

impl AccountStore {
    pub fn empty(dirs: &Dirs) -> Self { Self { file: AccountFile::default(), dirs: dirs.clone() } }
    pub fn load(dirs: &Dirs) -> Result<Self> { Ok(Self { file: load_json(&dirs.accounts_file())?, dirs: dirs.clone() }) }
    pub fn save(&self) -> Result<()> { save_json(&self.dirs.accounts_file(), &self.file) }

    pub fn add_offline(&mut self, name: &str) -> Result<String> {
        validate_offline_name(name)?;
        if self.file.accounts.iter().any(|a| a.kind == AccountKind::Offline && a.username.eq_ignore_ascii_case(name)) {
            return Err(Error::invalid("An offline profile with that name already exists."));
        }
        let id = format!("offline-{}", name.to_lowercase());
        self.file.accounts.push(Account { id: id.clone(), kind: AccountKind::Offline, username: name.into(), uuid: offline_uuid(name) });
        if self.file.selected.is_none() { self.file.selected = Some(id.clone()); }
        self.save()?;
        Ok(id)
    }

    pub fn upsert_microsoft(&mut self, uuid: &str, username: &str) -> Result<String> {
        let id = format!("ms-{}", uuid.replace('-', ""));
        let acc = Account { id: id.clone(), kind: AccountKind::Microsoft, username: username.into(), uuid: dash_uuid(uuid) };
        match self.file.accounts.iter_mut().find(|a| a.id == id) {
            Some(a) => *a = acc,
            None => self.file.accounts.push(acc),
        }
        self.file.selected = Some(id.clone());
        self.save()?;
        Ok(id)
    }

    pub fn remove(&mut self, id: &str, secrets: &dyn SecretStore) -> Result<()> {
        self.file.accounts.retain(|a| a.id != id);
        let _ = secrets.delete(&format!("{id}.refresh"));
        let _ = secrets.delete(&format!("{id}.mc"));
        if self.file.selected.as_deref() == Some(id) { self.file.selected = self.file.accounts.first().map(|a| a.id.clone()); }
        self.save()
    }
    pub fn selected(&self) -> Option<&Account> { self.file.selected.as_deref().and_then(|id| self.file.accounts.iter().find(|a| a.id == id)) }
}

// ---------------------------------------------------------------- Microsoft sign-in

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const MC_ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone)]
pub struct McSession {
    pub uuid: String,
    pub username: String,
    pub mc_access_token: String,
    pub mc_expires_in: u64,
    pub ms_refresh_token: String,
}

pub struct MicrosoftAuth {
    /// Azure application (client) ID. Use your own registration; Mojang must approve it for the
    /// Minecraft services API. The launcher does not ship a borrowed ID.
    pub client_id: String,
    http: reqwest::Client,
}

fn field(v: &Value, k: &str) -> Result<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
        .ok_or_else(|| Error::Auth(format!("unexpected response: missing '{k}'")))
}

impl MicrosoftAuth {
    pub fn new(client_id: &str) -> Result<Self> {
        if client_id.trim().is_empty() {
            return Err(Error::Auth("No Microsoft application (client) ID configured. Enter one in Accounts → Microsoft settings.".into()));
        }
        Ok(Self { client_id: client_id.trim().into(), http: crate::download::http_client()? })
    }

    pub async fn start_device_code(&self) -> Result<DeviceCode> {
        let v: Value = self.http.post(DEVICE_CODE_URL)
            .form(&[("client_id", self.client_id.as_str()), ("scope", SCOPE)])
            .send().await?.json().await?;
        if let Some(e) = v.get("error_description").and_then(|x| x.as_str()) { return Err(Error::Auth(e.to_string())); }
        Ok(DeviceCode {
            user_code: field(&v, "user_code")?,
            device_code: field(&v, "device_code")?,
            verification_uri: field(&v, "verification_uri")?,
            expires_in: v.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(900),
            interval: v.get("interval").and_then(|x| x.as_u64()).unwrap_or(5),
        })
    }

    /// Polls until the user finishes signing in, the code expires, or `cancel` returns true.
    pub async fn poll_device_code(&self, dc: &DeviceCode, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(String, String)> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(dc.expires_in);
        let mut interval = dc.interval.max(1);
        loop {
            if cancel() { return Err(Error::Cancelled); }
            if std::time::Instant::now() > deadline { return Err(Error::Auth("The sign-in code expired. Start again.".into())); }
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            let v: Value = self.http.post(TOKEN_URL)
                .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ("client_id", self.client_id.as_str()), ("device_code", dc.device_code.as_str())])
                .send().await?.json().await?;
            if let Some(at) = v.get("access_token").and_then(|x| x.as_str()) {
                let rt = field(&v, "refresh_token")?;
                logs::register_secret(at); logs::register_secret(&rt);
                return Ok((at.to_string(), rt));
            }
            match v.get("error").and_then(|x| x.as_str()) {
                Some("authorization_pending") => {}
                Some("slow_down") => interval += 5,
                Some(other) => return Err(Error::Auth(v.get("error_description").and_then(|x| x.as_str()).unwrap_or(other).to_string())),
                None => return Err(Error::Auth("unexpected token response".into())),
            }
        }
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<(String, String)> {
        let v: Value = self.http.post(TOKEN_URL)
            .form(&[("grant_type", "refresh_token"), ("client_id", self.client_id.as_str()),
                    ("refresh_token", refresh_token), ("scope", SCOPE)])
            .send().await?.json().await?;
        let at = v.get("access_token").and_then(|x| x.as_str())
            .ok_or_else(|| Error::Auth("Session expired. Sign in again.".into()))?;
        let rt = field(&v, "refresh_token")?;
        logs::register_secret(at); logs::register_secret(&rt);
        Ok((at.to_string(), rt))
    }

    /// Microsoft token -> Xbox Live -> XSTS -> Minecraft token -> ownership check -> profile.
    pub async fn minecraft_login(&self, ms_access: &str, ms_refresh: &str) -> Result<McSession> {
        let xbl: Value = self.http.post(XBL_URL)
            .json(&json!({"Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={ms_access}")},
                          "RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"}))
            .send().await?.error_for_status()?.json().await?;
        let xbl_token = field(&xbl, "Token")?;
        let uhs = xbl["DisplayClaims"]["xui"][0]["uhs"].as_str()
            .ok_or_else(|| Error::Auth("Xbox Live response missing user hash".into()))?.to_string();

        let r = self.http.post(XSTS_URL)
            .json(&json!({"Properties":{"SandboxId":"RETAIL","UserTokens":[xbl_token]},
                          "RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"}))
            .send().await?;
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            let v: Value = r.json().await.unwrap_or(Value::Null);
            return Err(Error::Auth(xsts_error(v.get("XErr").and_then(|x| x.as_u64()))));
        }
        let xsts: Value = r.error_for_status()?.json().await?;
        let xsts_token = field(&xsts, "Token")?;

        let mc: Value = self.http.post(MC_LOGIN_URL)
            .json(&json!({"identityToken": format!("XBL3.0 x={uhs};{xsts_token}")}))
            .send().await?.error_for_status()?.json().await?;
        let mc_token = field(&mc, "access_token")?;
        logs::register_secret(&mc_token);

        let ent: Value = self.http.get(MC_ENTITLEMENTS_URL).bearer_auth(&mc_token).send().await?.error_for_status()?.json().await?;
        if ent.get("items").and_then(|i| i.as_array()).map_or(true, |a| a.is_empty()) {
            return Err(Error::Auth("This Microsoft account does not own Minecraft: Java Edition.".into()));
        }
        let prof: Value = self.http.get(MC_PROFILE_URL).bearer_auth(&mc_token).send().await?.error_for_status()?.json().await?;
        Ok(McSession {
            uuid: dash_uuid(&field(&prof, "id")?),
            username: field(&prof, "name")?,
            mc_access_token: mc_token,
            mc_expires_in: mc.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(86400),
            ms_refresh_token: ms_refresh.to_string(),
        })
    }
}

fn xsts_error(code: Option<u64>) -> String {
    match code {
        Some(2148916233) => "This Microsoft account has no Xbox profile. Create one at xbox.com first.".into(),
        Some(2148916235) => "Xbox Live is not available in this account's country/region.".into(),
        Some(2148916236) | Some(2148916237) => "This account needs adult verification on the Xbox site.".into(),
        Some(2148916238) => "This is a child account; add it to a Microsoft family to play online.".into(),
        _ => "Xbox sign-in was refused.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_uuid_matches_vanilla_for_known_name() {
        // Vanilla's OfflinePlayer:Notch UUID.
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }
    #[test]
    fn name_rules() {
        assert!(validate_offline_name("Steve_1").is_ok());
        assert!(validate_offline_name("ab").is_err());
        assert!(validate_offline_name("has space").is_err());
    }
    #[test]
    fn dashes() { assert_eq!(dash_uuid("b50ad385829d3141a2167e7d7539ba7f"), "b50ad385-829d-3141-a216-7e7d7539ba7f"); }
}
