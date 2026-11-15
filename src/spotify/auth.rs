use super::transport::{Http, Result};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;

pub const REDIRECT: &str = "http://127.0.0.1:8888/callback";
const SCOPES: &str = "playlist-read-private playlist-read-collaborative user-read-playback-state user-modify-playback-state";

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub client_id: String,
    pub token: Option<Token>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn state_path() -> Result<PathBuf> {
    // Deliberately outside the project / MEGA sync and never checked into Git.
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/state/soffashell/spotify.json"))
}

pub fn load() -> Result<Saved> {
    let path = state_path()?;
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "Spotify settings are unreadable. Sign in again.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
        Err(_) => Err("Could not read local Spotify settings.".into()),
    }
}

pub fn save(saved: &Saved) -> Result<()> {
    save_at(&state_path()?, saved)
}

fn save_at(path: &std::path::Path, saved: &Saved) -> Result<()> {
    let directory = path.parent().ok_or("Invalid settings directory")?;
    fs::create_dir_all(directory).map_err(|_| "Could not create Spotify settings directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Could not protect Spotify settings")?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| "Could not create Spotify settings file")?;
        let bytes = serde_json::to_vec(saved).map_err(|_| "Could not encode Spotify settings")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not write Spotify settings")?;
        fs::rename(&temporary, path).map_err(|_| "Could not save Spotify settings")?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn random_string() -> Result<String> {
    let output = Command::new("openssl")
        .args(["rand", "32"])
        .output()
        .map_err(|_| "OpenSSL is required for secure Spotify sign-in")?;
    if !output.status.success() || output.stdout.len() != 32 {
        return Err("Could not generate sign-in randomness".into());
    }
    Ok(BASE64_URL_SAFE_NO_PAD.encode(&output.stdout))
}

fn challenge(verifier: &str) -> Result<String> {
    let mut child = Command::new("openssl")
        .args(["dgst", "-sha256", "-binary"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "OpenSSL is required for secure Spotify sign-in")?;
    child
        .stdin
        .take()
        .ok_or("Could not prepare PKCE")?
        .write_all(verifier.as_bytes())
        .map_err(|_| "Could not hash PKCE verifier")?;
    let output = child
        .wait_with_output()
        .map_err(|_| "Could not finish PKCE")?;
    if !output.status.success() || output.stdout.len() != 32 {
        return Err("Could not hash PKCE verifier".into());
    }
    Ok(BASE64_URL_SAFE_NO_PAD.encode(&output.stdout))
}

fn callback(target: &str, expected_state: &str) -> Result<String> {
    let url = Url::parse(&format!("http://127.0.0.1:8888{target}"))
        .map_err(|_| "Invalid sign-in callback")?;
    if url.path() != "/callback" {
        return Err("Unexpected callback path".into());
    }
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    if params.get("state").map(|s| s.as_ref()) != Some(expected_state) {
        return Err("Sign-in state did not match. Please try again.".into());
    }
    if params.contains_key("error") {
        return Err("Spotify sign-in was declined. You can try again.".into());
    }
    params
        .get("code")
        .map(|s| s.to_string())
        .ok_or_else(|| "No authorization code received".into())
}

pub fn login(http: &dyn Http, client_id: &str, cancelled: &Arc<AtomicBool>) -> Result<Token> {
    if client_id.len() != 32 || !client_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Enter the 32-character Client ID from your Spotify Developer app. No secret is needed.".into());
    }
    let listener = TcpListener::bind("127.0.0.1:8888").map_err(|_| {
        "Could not listen on port 8888. Close another sign-in attempt and retry.".to_owned()
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "Could not prepare sign-in listener")?;
    let verifier = random_string()?;
    let state = random_string()?;
    let mut url = Url::parse("https://accounts.spotify.com/authorize").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("scope", SCOPES),
        ("state", &state),
        ("code_challenge_method", "S256"),
        ("code_challenge", &challenge(&verifier)?),
    ]);
    webbrowser::open(url.as_str())
        .map_err(|_| "Could not open your browser for Spotify sign-in")?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if cancelled.load(Ordering::SeqCst) {
            return Err("Spotify sign-in cancelled.".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 512];
                while bytes.len() < 8192 && !bytes.windows(2).any(|pair| pair == b"\r\n") {
                    let Ok(count) = stream.read(&mut buffer) else {
                        break;
                    };
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let request = String::from_utf8_lossy(&bytes);
                let Some(target) = request
                    .lines()
                    .next()
                    .and_then(|line| line.strip_prefix("GET "))
                    .and_then(|line| line.split_whitespace().next())
                else {
                    continue;
                };
                if !target.starts_with("/callback?") {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                }
                let code = callback(target, &state);
                let text = if code.is_ok() {
                    "Sign-in received. Return to SoffaShell."
                } else {
                    "Sign-in failed. Return to SoffaShell and try again."
                };
                let reply = format!(
                    "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    if code.is_ok() {
                        "200 OK"
                    } else {
                        "400 Bad Request"
                    },
                    text.len()
                );
                let _ = stream.write_all(reply.as_bytes());
                let code = code?;
                if cancelled.load(Ordering::SeqCst) {
                    return Err("Spotify sign-in cancelled.".into());
                }
                return exchange(
                    http,
                    &[
                        ("grant_type", "authorization_code"),
                        ("code", &code),
                        ("client_id", client_id),
                        ("redirect_uri", REDIRECT),
                        ("code_verifier", &verifier),
                    ],
                    None,
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(_) => return Err("Could not receive Spotify sign-in".into()),
        }
    }
    Err("Spotify sign-in timed out. Select Connect to try again.".into())
}

pub fn refresh(http: &dyn Http, client_id: &str, token: &Token) -> Result<Token> {
    exchange(
        http,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &token.refresh_token),
            ("client_id", client_id),
        ],
        Some(&token.refresh_token),
    )
}

fn exchange(
    http: &dyn Http,
    fields: &[(&str, &str)],
    previous_refresh: Option<&str>,
) -> Result<Token> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let response = http.request(
        "POST",
        "https://accounts.spotify.com/api/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some(&body),
    )?;
    if response.status != 200 {
        return Err(format!(
            "Spotify sign-in failed (HTTP {}). Check your Client ID, redirect URI, and app access, then reconnect.",
            response.status
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|_| "Invalid Spotify sign-in response")?;
    Ok(Token {
        access_token: value["access_token"]
            .as_str()
            .ok_or("Missing access token")?
            .into(),
        refresh_token: value["refresh_token"]
            .as_str()
            .or(previous_refresh)
            .ok_or("Missing refresh token")?
            .into(),
        expires_at: now() + value["expires_in"].as_u64().unwrap_or(3600),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_matches_rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").unwrap(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(random_string().unwrap().len(), 43);
    }
    #[test]
    fn callback_validates_state_and_denial() {
        assert_eq!(
            callback("/callback?code=hello&state=good", "good").unwrap(),
            "hello"
        );
        assert!(callback("/callback?code=hello&state=bad", "good").is_err());
        assert!(callback("/callback?error=access_denied&state=good", "good").is_err());
        assert!(callback("/wrong?code=hello&state=good", "good").is_err());
    }

    #[test]
    fn credentials_are_written_outside_repo_with_owner_only_permissions() {
        let directory = std::env::temp_dir().join(format!(
            "soffashell-auth-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("spotify.json");
        let saved = Saved {
            client_id: "test-client".into(),
            token: Some(Token {
                access_token: "fake-access".into(),
                refresh_token: "fake-refresh".into(),
                expires_at: 100,
            }),
        };
        save_at(&path, &saved).unwrap();
        let restored: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.token.unwrap().refresh_token, "fake-refresh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        save_at(
            &path,
            &Saved {
                client_id: "test-client".into(),
                token: None,
            },
        )
        .unwrap();
        let restored: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(restored.token.is_none());
        fs::remove_dir_all(directory).unwrap();
    }
}
