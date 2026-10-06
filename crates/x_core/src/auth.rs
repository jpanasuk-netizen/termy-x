use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use crate::config::XConfig;
use crate::io::{Http, HttpRequest};

pub const SCOPES: &str = "tweet.read users.read like.read follows.read offline.access tweet.write";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

pub fn generate_pkce() -> Pkce {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    Pkce {
        challenge: URL_SAFE_NO_PAD.encode(digest),
        verifier,
    }
}

pub fn authorize_url(config: &XConfig, state: &str, challenge: &str) -> Result<String, String> {
    let client_id = config
        .client_id()
        .ok_or_else(|| format!("set {} to your X app client id", config.client_id_env))?;
    Ok(format!(
        "https://twitter.com/i/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        crate::text::percent_encode(&client_id),
        crate::text::percent_encode(&config.redirect_uri),
        crate::text::percent_encode(SCOPES),
        crate::text::percent_encode(state),
        crate::text::percent_encode(challenge)
    ))
}

pub fn exchange_code(
    config: &XConfig,
    http: &dyn Http,
    code: &str,
    verifier: &str,
) -> Result<TokenSet, String> {
    let client_id = config
        .client_id()
        .ok_or_else(|| "missing client id".to_string())?;
    let body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        crate::text::percent_encode(code),
        crate::text::percent_encode(&config.redirect_uri),
        crate::text::percent_encode(&client_id),
        crate::text::percent_encode(verifier)
    );
    let response = http.request(HttpRequest {
        method: "POST",
        url: "https://api.x.com/2/oauth2/token".into(),
        body: Some(body),
        bearer: None,
        content_type: Some("application/x-www-form-urlencoded".into()),
    })?;
    parse_token(&response)
}

pub fn store_token(config: &XConfig, token: &TokenSet) -> Result<(), String> {
    let json = serde_json::to_string(token).map_err(|error| error.to_string())?;
    if config.token_store == "keychain" {
        match keyring::Entry::new("termy-x", "x-oauth") {
            Ok(entry) => {
                if entry.set_password(&json).is_ok() {
                    return Ok(());
                }
            }
            Err(_) => {}
        }
    }
    write_private_file(&config.token_path(), &json)
}

pub fn load_access_token(config: &XConfig) -> Result<String, String> {
    if config.token_store == "keychain" {
        if let Ok(entry) = keyring::Entry::new("termy-x", "x-oauth") {
            if let Ok(json) = entry.get_password() {
                if let Ok(token) = serde_json::from_str::<TokenSet>(&json) {
                    return Ok(token.access_token);
                }
            }
        }
    }
    let json = fs::read_to_string(config.token_path()).map_err(|error| error.to_string())?;
    parse_token(&json).map(|token| token.access_token)
}

pub fn write_private_file(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    file.write_all(contents.as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn parse_token(body: &str) -> Result<TokenSet, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let access_token = value
        .get("access_token")
        .and_then(|token| token.as_str())
        .ok_or_else(|| "token response missing access_token".to_string())?
        .to_string();
    Ok(TokenSet {
        access_token,
        refresh_token: value
            .get("refresh_token")
            .and_then(|token| token.as_str())
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn pkce_challenge_is_s256_and_authorize_url_asks_for_write() {
        let pkce = generate_pkce();
        let digest = Sha256::digest(pkce.verifier.as_bytes());
        assert_eq!(pkce.challenge, URL_SAFE_NO_PAD.encode(digest));
        assert!(!pkce.challenge.contains('='));
        let mut config = XConfig::default();
        let env_key = config.client_id_env.clone();
        // set_var / remove_var are process-global and unsafe as of Rust 1.87.
        unsafe {
            std::env::set_var(&env_key, "client-test");
        }
        config.official_enabled = true;
        let url = authorize_url(&config, "state", &pkce.challenge).unwrap();
        unsafe {
            std::env::remove_var(&env_key);
        }
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("tweet.write"));
        assert!(url.contains("tweet.read"));
        assert!(!url.contains("client_secret"));
    }

    #[test]
    fn token_file_is_owner_readable_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-tokens.json");
        write_private_file(&path, r#"{"access_token":"secret","refresh_token":null}"#).unwrap();
        #[cfg(unix)]
        {
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("secret"));
    }
}
