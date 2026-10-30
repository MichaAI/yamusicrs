//! OAuth: device code flow (как у Android-приложения Музыки) и ручной ввод токена.

use std::path::Path;

use serde::{Deserialize, Serialize};

const OAUTH: &str = "https://oauth.yandex.ru";
const CLIENT_ID: &str = "23cabbbdc6cd418abb4b39c32c41195d";
const CLIENT_SECRET: &str = "53bc75238f0c4d08a118e51fe9203300";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix-время истечения, если известно.
    pub expires_at: Option<i64>,
}

impl StoredToken {
    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(self)?)?;
        f.sync_all()?;
        std::fs::rename(tmp, path)
    }

    /// Пора ли обновить токен (меньше 30 дней до истечения).
    pub fn needs_refresh(&self) -> bool {
        self.refresh_token.is_some()
            && self
                .expires_at
                .is_some_and(|t| t - chrono::Utc::now().timestamp() < 30 * 24 * 3600)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    #[serde(default = "five")]
    pub interval: u64,
    pub expires_in: u64,
}

fn five() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

pub enum Poll {
    Pending,
    Done(StoredToken),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("сеть: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    OAuth(String),
}

fn device_id() -> String {
    use rand::Rng;
    rand::rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(10)
        .map(char::from)
        .collect()
}

pub async fn request_device_code(http: &reqwest::Client) -> Result<DeviceCode, AuthError> {
    let resp = http
        .post(format!("{OAUTH}/device/code"))
        .form(&[
            ("client_id", CLIENT_ID),
            ("device_id", &device_id()),
            ("device_name", "yamusicrs"),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(AuthError::OAuth(format!(
            "device/code: {}",
            resp.text().await.unwrap_or_default()
        )));
    }
    Ok(resp.json().await?)
}

fn into_token(r: TokenResponse) -> Result<Poll, AuthError> {
    match (r.access_token, r.error) {
        (Some(access_token), _) => Ok(Poll::Done(StoredToken {
            access_token,
            refresh_token: r.refresh_token,
            expires_at: r.expires_in.map(|e| chrono::Utc::now().timestamp() + e),
        })),
        (None, Some(e)) if e == "authorization_pending" || e == "slow_down" => Ok(Poll::Pending),
        (None, e) => Err(AuthError::OAuth(
            r.error_description.or(e).unwrap_or_else(|| "неизвестная ошибка".into()),
        )),
    }
}

pub async fn poll_device_token(http: &reqwest::Client, code: &DeviceCode) -> Result<Poll, AuthError> {
    let r: TokenResponse = http
        .post(format!("{OAUTH}/token"))
        .form(&[
            ("grant_type", "device_code"),
            ("code", &code.device_code),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ])
        .send()
        .await?
        .json()
        .await?;
    into_token(r)
}

pub async fn refresh(http: &reqwest::Client, refresh_token: &str) -> Result<StoredToken, AuthError> {
    let r: TokenResponse = http
        .post(format!("{OAUTH}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ])
        .send()
        .await?
        .json()
        .await?;
    match into_token(r)? {
        Poll::Done(mut t) => {
            t.refresh_token.get_or_insert_with(|| refresh_token.to_string());
            Ok(t)
        }
        Poll::Pending => Err(AuthError::OAuth("refresh: неожиданный pending".into())),
    }
}

/// Принимает голый токен или целиком URL редиректа с `#access_token=...`.
pub fn parse_pasted_token(input: &str) -> Option<String> {
    let s = input.trim();
    if let Some(idx) = s.find("access_token=") {
        let rest = &s[idx + "access_token=".len()..];
        let tok: String = rest.chars().take_while(|c| *c != '&').collect();
        return (!tok.is_empty()).then_some(tok);
    }
    let ok = s.len() >= 20 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| s.to_string())
}

/// Ссылка для ручного получения токена через браузер.
pub fn manual_token_url() -> String {
    format!("{OAUTH}/authorize?response_type=token&client_id={CLIENT_ID}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_parsing() {
        assert_eq!(
            parse_pasted_token("https://music.yandex.ru/#access_token=y0_AgAAAABBBBccccDDDDeeee&token_type=bearer"),
            Some("y0_AgAAAABBBBccccDDDDeeee".into())
        );
        assert_eq!(
            parse_pasted_token("  y0_AgAAAABBBBccccDDDDeeee \n"),
            Some("y0_AgAAAABBBBccccDDDDeeee".into())
        );
        assert_eq!(parse_pasted_token("hello world"), None);
    }
}
