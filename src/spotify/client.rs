use super::{
    auth::{self, Saved},
    model::{self, Entry, Page, Playback},
    transport::{Http, Result},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use url::Url;

const API: &str = "https://api.spotify.com/v1";

pub struct Client {
    pub saved: Saved,
    http: Box<dyn Http>,
    retry_at: Option<Instant>,
    store: fn(&Saved) -> Result<()>,
}

impl Client {
    pub fn new(saved: Saved, http: Box<dyn Http>) -> Self {
        Self {
            saved,
            http,
            retry_at: None,
            store: auth::save,
        }
    }

    pub fn connect(
        &mut self,
        client_id: &str,
        cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<()> {
        let token = auth::login(self.http.as_ref(), client_id, cancelled)?;
        self.saved = Saved {
            client_id: client_id.into(),
            token: Some(token),
        };
        (self.store)(&self.saved)
    }

    pub fn disconnect(&mut self) -> Result<()> {
        self.saved.token = None;
        (self.store)(&self.saved)
    }

    fn refresh(&mut self) -> Result<()> {
        let previous = self
            .saved
            .token
            .as_ref()
            .ok_or("Connect your Spotify account first.")?;
        let token = auth::refresh(self.http.as_ref(), &self.saved.client_id, previous)?;
        self.saved.token = Some(token);
        (self.store)(&self.saved)
    }

    fn request(&mut self, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value> {
        if let Some(retry_at) = self.retry_at.filter(|time| *time > Instant::now()) {
            return Err(format!(
                "Spotify rate limit. Try again in {} seconds.",
                retry_at.duration_since(Instant::now()).as_secs() + 1
            ));
        }
        let address = if endpoint.starts_with("https://") {
            endpoint.to_owned()
        } else {
            format!("{API}{endpoint}")
        };
        let url = Url::parse(&address).map_err(|_| "Invalid Spotify API URL")?;
        if url.scheme() != "https"
            || url.host_str() != Some("api.spotify.com")
            || !url.path().starts_with("/v1/")
        {
            return Err("Rejected an unexpected Spotify API address".into());
        }
        if self
            .saved
            .token
            .as_ref()
            .ok_or("Connect your Spotify account first.")?
            .expires_at
            <= auth::now() + 60
        {
            self.refresh()?;
        }
        for attempt in 0..2 {
            let token = &self
                .saved
                .token
                .as_ref()
                .ok_or("Connect your Spotify account first.")?
                .access_token;
            let bearer = format!("Bearer {token}");
            let body_text = body.as_ref().map(Value::to_string);
            let response = self.http.request(
                method,
                url.as_str(),
                &[
                    ("Authorization", &bearer),
                    ("Content-Type", "application/json"),
                ],
                body_text.as_deref(),
            )?;
            match response.status {
                200..=299 => return if response.body.is_empty() { Ok(Value::Null) } else { serde_json::from_slice(&response.body).map_err(|_| "Spotify returned an unreadable response".into()) },
                401 if attempt == 0 => { self.refresh()?; }
                401 => return Err("Spotify sign-in expired. Reconnect your account.".into()),
                403 if url.path().starts_with("/v1/playlists/") => return Err("Spotify limits track lists to playlists you own or collaborate on. You can still try Play playlist.".into()),
                403 => return Err("Spotify denied this action. Check Premium, Developer app access, and playback permissions.".into()),
                404 if url.path().contains("/me/player") => return Err("No active Spotify player. Open Spotify, then select a player in Devices.".into()),
                429 => {
                    let seconds = response.retry_after.clamp(1, 3600);
                    self.retry_at = Some(Instant::now() + Duration::from_secs(seconds));
                    return Err(format!("Spotify rate limit. Wait {seconds} seconds before trying again."));
                }
                status => return Err(format!("Spotify request failed (HTTP {status}). Please try again.")),
            }
        }
        Err("Could not authenticate with Spotify".into())
    }

    pub fn playlists(&mut self, next: Option<&str>) -> Result<Page> {
        self.request("GET", next.unwrap_or("/me/playlists?limit=50"), None)
            .map(|value| model::playlists(&value))
    }

    pub fn tracks(&mut self, id: &str, next: Option<&str>) -> Result<Page> {
        if !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err("Invalid Spotify playlist ID".into());
        }
        self.request(
            "GET",
            next.unwrap_or(&format!("/playlists/{id}/items?limit=50")),
            None,
        )
        .map(|value| model::tracks(&value))
    }

    pub fn devices(&mut self) -> Result<Vec<Entry>> {
        self.request("GET", "/me/player/devices", None)
            .map(|value| model::devices(&value))
    }

    pub fn playback(&mut self) -> Result<Playback> {
        self.request("GET", "/me/player", None)
            .map(|value| Playback::parse(&value))
    }

    pub fn play(&mut self, playlist: &str, position: Option<usize>) -> Result<()> {
        let mut body = json!({"context_uri": playlist});
        if let Some(position) = position {
            body["offset"] = json!({"position": position});
        }
        self.request("PUT", "/me/player/play", Some(body))
            .map(|_| ())
    }

    pub fn transfer(&mut self, id: &str) -> Result<()> {
        self.request(
            "PUT",
            "/me/player",
            Some(json!({"device_ids": [id], "play": false})),
        )
        .map(|_| ())
    }

    pub fn control(&mut self, action: &str) -> Result<()> {
        let (method, path) = match action {
            "pause" => ("PUT", "/me/player/pause"),
            "resume" => ("PUT", "/me/player/play"),
            "next" => ("POST", "/me/player/next"),
            "previous" => ("POST", "/me/player/previous"),
            _ => return Err("Unknown playback control".into()),
        };
        self.request(method, path, None).map(|_| ())
    }

    pub fn artwork(&self, address: &str) -> Result<(Vec<u8>, u32, u32)> {
        let url = Url::parse(address).map_err(|_| "Invalid artwork URL")?;
        let host = url.host_str().unwrap_or_default();
        if url.scheme() != "https" || !(host == "scdn.co" || host.ends_with(".scdn.co")) {
            return Err("Unsupported Spotify artwork host".into());
        }
        let response = self.http.request("GET", url.as_str(), &[], None)?;
        if response.status != 200 {
            return Err("Could not load Spotify artwork".into());
        }
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&response.body))
            .with_guessed_format()
            .map_err(|_| "Invalid artwork")?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| "Could not decode artwork")?
            .thumbnail(600, 600)
            .into_rgba8();
        let (width, height) = image.dimensions();
        Ok((image.into_raw(), width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spotify::{auth::Token, transport::Response};
    use std::sync::{Arc, Mutex};
    struct Fake {
        calls: Arc<Mutex<usize>>,
        status: u16,
    }
    impl Http for Fake {
        fn request(
            &self,
            _: &str,
            _: &str,
            _: &[(&str, &str)],
            _: Option<&str>,
        ) -> Result<Response> {
            *self.calls.lock().unwrap() += 1;
            Ok(Response {
                status: self.status,
                body: b"{}".to_vec(),
                retry_after: 2,
            })
        }
    }
    fn client(status: u16) -> (Client, Arc<Mutex<usize>>) {
        let calls = Arc::new(Mutex::new(0));
        let saved = Saved {
            client_id: "test".into(),
            token: Some(Token {
                access_token: "not-a-real-token".into(),
                refresh_token: "test".into(),
                expires_at: u64::MAX,
            }),
        };
        (
            Client::new(
                saved,
                Box::new(Fake {
                    calls: calls.clone(),
                    status,
                }),
            ),
            calls,
        )
    }
    #[test]
    fn pagination_cannot_send_token_to_another_host() {
        let (mut client, calls) = client(200);
        assert!(
            client
                .playlists(Some("https://example.com/stolen"))
                .is_err()
        );
        assert_eq!(*calls.lock().unwrap(), 0);
    }
    #[test]
    fn rate_limit_prevents_repeated_requests() {
        let (mut client, calls) = client(429);
        assert!(client.playlists(None).is_err());
        assert!(client.playlists(None).is_err());
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    struct RefreshFlow(Mutex<usize>);
    impl Http for RefreshFlow {
        fn request(
            &self,
            method: &str,
            url: &str,
            headers: &[(&str, &str)],
            body: Option<&str>,
        ) -> Result<Response> {
            let mut step = self.0.lock().unwrap();
            *step += 1;
            let (status, body) = match *step {
                1 => {
                    assert!(headers.contains(&("Authorization", "Bearer old")));
                    (401, json!({}))
                }
                2 => {
                    assert_eq!(method, "POST");
                    assert_eq!(url, "https://accounts.spotify.com/api/token");
                    assert!(body.unwrap().contains("grant_type=refresh_token"));
                    (
                        200,
                        json!({"access_token":"new","refresh_token":"rotated","expires_in":3600}),
                    )
                }
                3 => {
                    assert!(headers.contains(&("Authorization", "Bearer new")));
                    (200, json!({"items":[],"next":null}))
                }
                _ => panic!("Unexpected extra request"),
            };
            Ok(Response {
                status,
                body: body.to_string().into_bytes(),
                retry_after: 0,
            })
        }
    }

    #[test]
    fn unauthorized_request_refreshes_rotated_token_then_retries_once() {
        let saved = Saved {
            client_id: "app".into(),
            token: Some(Token {
                access_token: "old".into(),
                refresh_token: "previous".into(),
                expires_at: u64::MAX,
            }),
        };
        let mut client = Client::new(saved, Box::new(RefreshFlow(Mutex::new(0))));
        // Keep this test independent of the user's actual state directory.
        client.store = |_| Ok(());
        assert!(client.playlists(None).is_ok());
        assert_eq!(
            client.saved.token.as_ref().unwrap().refresh_token,
            "rotated"
        );
    }
}
