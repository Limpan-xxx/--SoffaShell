use crate::{
    MusicRow, Shell,
    spotify::{
        auth,
        client::Client,
        model::{Entry, Page, Playback},
        transport::{Curl, Result},
    },
};
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

enum Job {
    Connect(String),
    Disconnect,
    Library(u64, Option<String>),
    Tracks(u64, String, Option<String>),
    Devices(u64),
    Playback,
    Play(String, Option<usize>),
    Control(String),
    Transfer(String),
    Art(String),
}
enum Event {
    Initial(auth::Saved, Option<String>),
    Connected(Result<()>, String),
    Disconnected(Result<()>),
    Library(u64, Result<Page>),
    Tracks(u64, Result<Page>),
    Devices(u64, Result<Vec<Entry>>),
    Playback(Result<Playback>),
    Action(Result<()>),
    Art(String, Result<(Vec<u8>, u32, u32)>),
}

fn worker(receiver: Receiver<Job>, sender: Sender<Event>, cancelled: Arc<AtomicBool>) {
    let (mut saved, warning) = match auth::load() {
        Ok(saved) => (saved, None),
        Err(error) => (auth::Saved::default(), Some(error)),
    };
    if let Ok(id) = std::env::var("SOFFASHELL_SPOTIFY_CLIENT_ID")
        && id != saved.client_id
    {
        saved.token = None;
        saved.client_id = id;
    }
    let _ = sender.send(Event::Initial(saved.clone(), warning));
    let mut client = Client::new(saved, Box::new(Curl));
    let mut covers: std::collections::HashMap<String, (Vec<u8>, u32, u32)> =
        std::collections::HashMap::new();
    while let Ok(job) = receiver.recv() {
        let event = match job {
            Job::Connect(id) => Event::Connected(client.connect(&id, &cancelled), id),
            Job::Disconnect => Event::Disconnected(client.disconnect()),
            Job::Library(generation, next) => {
                Event::Library(generation, client.playlists(next.as_deref()))
            }
            Job::Tracks(generation, id, next) => {
                Event::Tracks(generation, client.tracks(&id, next.as_deref()))
            }
            Job::Devices(generation) => Event::Devices(generation, client.devices()),
            Job::Playback => Event::Playback(client.playback()),
            Job::Play(uri, position) => Event::Action(client.play(&uri, position)),
            Job::Control(action) => Event::Action(client.control(&action)),
            Job::Transfer(id) => Event::Action(client.transfer(&id)),
            Job::Art(url) => {
                let result = if let Some(cover) = covers.get(&url) {
                    Ok(cover.clone())
                } else {
                    let result = client.artwork(&url);
                    if let Ok(cover) = &result {
                        if covers.len() >= 8 {
                            covers.clear();
                        }
                        covers.insert(url.clone(), cover.clone());
                    }
                    result
                };
                Event::Art(url, result)
            }
        };
        if sender.send(event).is_err() {
            break;
        }
    }
}

#[derive(Clone)]
enum Row {
    Playlist(Entry),
    Track(Entry),
    Device(Entry),
    PlayPlaylist,
    More(String),
    Control(&'static str),
    Connect,
    Link,
}

struct Controller {
    desktop: Rc<dyn Fn()>,
    ui: slint::Weak<Shell>,
    sender: Sender<Job>,
    receiver: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    demo: bool,
    initialized: bool,
    connected: bool,
    signing_in: bool,
    busy: bool,
    poll_pending: bool,
    generation: u64,
    page: &'static str,
    rows: Vec<Row>,
    selected: usize,
    menu: usize,
    focus: i32,
    playlist: Option<Entry>,
    library: Vec<Entry>,
    library_next: Option<String>,
    library_selection: usize,
    playback: Playback,
    last_poll: Instant,
    playback_at: Instant,
    cover_url: String,
    cover_at: Instant,
    cover_requested: bool,
}

impl Controller {
    fn status(&self, message: &str) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_music_status(message.into());
        }
    }
    fn send(&self, job: Job) {
        let _ = self.sender.send(job);
    }

    fn change_page(&mut self, page: &'static str) {
        self.generation += 1;
        self.page = page;
        self.rows.clear();
        self.selected = 0;
        self.busy = false;
        self.focus = 1;
        self.status("");
        if let Some(ui) = self.ui.upgrade() {
            ui.invoke_focus_navigation();
        }
    }

    fn open(&mut self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_music_visible(true);
        }
        if self.demo || self.connected {
            self.library();
        } else {
            self.setup();
        }
    }

    fn setup(&mut self) {
        self.change_page("setup");
        self.menu = 3;
        self.rows = vec![Row::Connect];
        self.sync();
        self.status("Create a Spotify Developer app, then paste its Client ID here. No client secret is needed.");
    }

    fn library(&mut self) {
        if !self.demo && !self.connected {
            self.setup();
            return;
        }
        self.change_page("library");
        self.menu = 0;
        if self.demo {
            self.library = demo_playlists();
            self.library_next = None;
        }
        self.rows = self.library.iter().cloned().map(Row::Playlist).collect();
        if let Some(next) = &self.library_next {
            self.rows.push(Row::More(next.clone()));
        }
        self.selected = self
            .library_selection
            .min(self.rows.len().saturating_sub(1));
        if self.library.is_empty() && !self.demo {
            self.busy = true;
            self.send(Job::Library(self.generation, None));
            self.status("Loading your playlists…");
        }
        self.sync();
    }

    fn tracks(&mut self, playlist: Entry) {
        self.library_selection = self.selected;
        self.change_page("tracks");
        self.menu = 0;
        self.playlist = Some(playlist.clone());
        self.rows.push(Row::PlayPlaylist);
        if self.demo {
            self.rows.extend(demo_tracks().into_iter().map(Row::Track));
        } else {
            self.busy = true;
            self.send(Job::Tracks(self.generation, playlist.id, None));
            self.status("Loading playlist tracks…");
        }
        self.sync();
    }

    fn devices(&mut self) {
        if !self.demo && !self.connected {
            self.setup();
            return;
        }
        self.change_page("devices");
        self.menu = 2;
        if self.demo {
            self.rows.push(Row::Device(Entry {
                id: "demo".into(),
                name: "Living room PC".into(),
                detail: "Computer · Demo".into(),
                playable: true,
                ..Entry::default()
            }));
        } else {
            self.busy = true;
            self.send(Job::Devices(self.generation));
            self.status("Finding Spotify players…");
        }
        self.sync();
    }

    fn now_playing(&mut self) {
        if !self.demo && !self.connected {
            self.setup();
            return;
        }
        self.change_page("now-playing");
        self.menu = 1;
        self.rows = vec![
            Row::Control("previous"),
            Row::Control("toggle"),
            Row::Control("next"),
            Row::Link,
        ];
        self.selected = 1;
        self.sync();
        self.poll();
    }

    fn poll(&mut self) {
        if !self.demo && self.connected && !self.poll_pending && !self.signing_in {
            self.poll_pending = true;
            self.last_poll = Instant::now();
            self.send(Job::Playback);
        }
    }

    fn action(&mut self, action: &str) {
        match action {
            "library" => self.library(),
            "now-playing" => self.now_playing(),
            "devices" => self.devices(),
            "account" => {
                if self.connected && !self.demo {
                    self.cancelled.store(true, Ordering::SeqCst);
                    self.send(Job::Disconnect);
                    self.status("Signing out…");
                } else {
                    self.setup();
                }
            }
            "connect" => {
                if self.demo {
                    self.status("Demo mode uses sample content. Restart without --demo-spotify to connect your account.");
                    return;
                }
                if self.signing_in {
                    self.cancelled.store(true, Ordering::SeqCst);
                    self.status("Cancelling sign-in…");
                    return;
                }
                if let Some(ui) = self.ui.upgrade() {
                    let id = ui.get_music_client_id().trim().to_owned();
                    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        self.status("Paste the 32-character Client ID from your Spotify Developer app. No secret is needed.");
                        return;
                    }
                    self.cancelled.store(false, Ordering::SeqCst);
                    self.signing_in = true;
                    self.send(Job::Connect(id));
                    self.status(
                        "Finish signing in in your browser, then return here. Escape cancels.",
                    );
                    self.sync();
                }
            }
            "developer" => {
                let _ = webbrowser::open("https://developer.spotify.com/dashboard");
            }
            "desktop" => {
                if self.demo {
                    self.status("Demo mode does not launch a Spotify player. Restart without --demo-spotify to use your account.");
                    return;
                }
                (self.desktop)();
                self.status(
                    "Spotify desktop app opened. Use Alt+Tab to return here, then refresh Devices.",
                );
            }
            "home" => {
                self.cancelled.store(true, Ordering::SeqCst);
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_music_visible(false);
                    ui.invoke_focus_navigation();
                }
            }
            "quit" => {
                self.cancelled.store(true, Ordering::SeqCst);
                self.playback = Playback::default();
                self.sync();
                self.action("home");
                if self.demo {
                    return;
                }
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_status("Closing Spotify…".into());
                }
                let ui = self.ui.clone();
                std::thread::spawn(move || {
                    let message = match crate::processes::quit_spotify() {
                        Ok(()) => "Spotify closed.".to_owned(),
                        Err(error) => format!("Could not close Spotify: {error}"),
                    };
                    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_status(message.into()));
                });
            }
            "refresh" => {
                if self.busy || self.signing_in {
                    return;
                }
                match self.page {
                    "library" => {
                        self.library.clear();
                        self.library_next = None;
                        self.library();
                    }
                    "tracks" => {
                        if let Some(playlist) = &self.playlist {
                            self.tracks(playlist.clone());
                        }
                    }
                    "devices" => self.devices(),
                    _ => self.poll(),
                }
            }
            "toggle" | "previous" | "next" => {
                if !self.connected && !self.demo {
                    self.setup();
                    return;
                }
                if self.demo {
                    if action == "toggle" {
                        self.playback.playing = !self.playback.playing;
                    }
                    self.playback_at = Instant::now();
                    self.sync();
                    self.status("Demo playback controls only; no audio is playing.");
                } else {
                    let command = if action == "toggle" {
                        if self.playback.playing {
                            "pause"
                        } else {
                            "resume"
                        }
                    } else {
                        action
                    };
                    self.send(Job::Control(command.into()));
                }
            }
            "link" => {
                let uri = if self.page == "tracks" {
                    self.playlist
                        .as_ref()
                        .map(|item| item.uri.as_str())
                        .unwrap_or("")
                } else {
                    &self.playback.uri
                };
                if let Some(url) = spotify_link(uri) {
                    let _ = webbrowser::open(&url);
                }
            }
            _ => {}
        }
    }

    fn navigate(&mut self, horizontal: i32, vertical: i32) {
        if horizontal != 0 {
            self.focus = if horizontal < 0 { 0 } else { 1 };
            if self.page == "setup"
                && self.focus == 1
                && let Some(ui) = self.ui.upgrade()
            {
                ui.invoke_focus_music_input();
            }
        } else if self.focus == 0 {
            self.menu = crate::navigation::next_selection(self.menu, vertical, 6);
        } else if !self.rows.is_empty() {
            self.selected =
                crate::navigation::next_selection(self.selected, vertical, self.rows.len());
        }
        self.sync();
    }

    fn activate(&mut self) {
        if self.focus == 0 {
            self.action(
                [
                    "library",
                    "now-playing",
                    "devices",
                    "account",
                    "desktop",
                    "home",
                ][self.menu],
            );
            return;
        }
        if self.busy || self.signing_in {
            if self.page == "setup" {
                self.action("connect");
            }
            return;
        }
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return;
        };
        match row {
            Row::Playlist(playlist) => self.tracks(playlist),
            Row::Track(track) => {
                if !track.playable {
                    self.status("This track is unavailable for playback on this account.");
                    return;
                }
                self.play(Some(track.position));
            }
            Row::PlayPlaylist => self.play(None),
            Row::More(next) => {
                self.busy = true;
                if self.page == "library" {
                    self.send(Job::Library(self.generation, Some(next)));
                } else if let Some(playlist) = &self.playlist {
                    self.send(Job::Tracks(
                        self.generation,
                        playlist.id.clone(),
                        Some(next),
                    ));
                }
                self.status("Loading more…");
                self.sync();
            }
            Row::Device(device) => {
                if !device.playable {
                    self.status("This Spotify player cannot be controlled. Choose another device.");
                    return;
                }
                if self.demo {
                    self.status("Demo player selected.");
                } else {
                    self.send(Job::Transfer(device.id));
                    self.status("Selecting player…");
                }
            }
            Row::Control(control) => self.action(control),
            Row::Connect => self.action("connect"),
            Row::Link => self.action("link"),
        }
    }

    fn play(&mut self, position: Option<usize>) {
        let Some(playlist) = &self.playlist else {
            return;
        };
        if self.demo {
            let tracks = demo_tracks();
            let track = &tracks[position.unwrap_or(0).min(tracks.len() - 1)];
            self.playback = Playback {
                name: track.name.clone(),
                artist: "SoffaShell Demo".into(),
                album: playlist.name.clone(),
                device: "Living room PC · Demo".into(),
                playing: true,
                duration_ms: 210000,
                ..Playback::default()
            };
            self.playback_at = Instant::now();
            self.now_playing();
            self.status("Demo mode: sample metadata, no audio playback.");
        } else {
            self.send(Job::Play(playlist.uri.clone(), position));
            self.status("Starting playback on your selected Spotify player…");
        }
    }

    fn back(&mut self) {
        if self.signing_in {
            self.cancelled.store(true, Ordering::SeqCst);
            self.status("Cancelling sign-in…");
            return;
        }
        if self.page == "tracks" || (self.page != "library" && (self.connected || self.demo)) {
            self.library();
        } else {
            self.action("home");
        }
    }

    fn sync(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_music_page(self.page.into());
        ui.set_music_menu(self.menu as i32);
        ui.set_music_focus(self.focus);
        ui.set_music_selected(self.selected as i32);
        ui.set_music_connected(self.connected);
        ui.set_music_demo(self.demo);
        ui.set_music_busy(self.busy || self.signing_in);
        ui.set_music_signing_in(self.signing_in);
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let (title, subtitle, enabled) = match row {
                    Row::Playlist(item) | Row::Track(item) | Row::Device(item) => {
                        (item.name.clone(), item.detail.clone(), item.playable)
                    }
                    Row::PlayPlaylist => (
                        "Play playlist".into(),
                        "Play on your active Spotify device".into(),
                        true,
                    ),
                    Row::More(_) => ("Load more".into(), "Show the next page".into(), true),
                    Row::Control(control) => (
                        match *control {
                            "previous" => "Previous track",
                            "next" => "Next track",
                            _ => {
                                if self.playback.playing {
                                    "Pause"
                                } else {
                                    "Play"
                                }
                            }
                        }
                        .into(),
                        "Playback control".into(),
                        true,
                    ),
                    Row::Connect => (
                        "Connect Spotify".into(),
                        "Sign in securely in your browser".into(),
                        true,
                    ),
                    Row::Link => (
                        "Open in Spotify".into(),
                        "View this track on Spotify".into(),
                        !self.playback.uri.is_empty(),
                    ),
                };
                MusicRow {
                    title: title.into(),
                    subtitle: subtitle.into(),
                    enabled,
                }
            })
            .collect::<Vec<_>>();
        ui.set_music_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
        let (title, detail, cover) = match self.page {
            "now-playing" => (
                if self.playback.name.is_empty() {
                    "Nothing playing".into()
                } else {
                    self.playback.name.clone()
                },
                format!("{}\n{}", self.playback.artist, self.playback.album),
                self.playback.image.clone(),
            ),
            "tracks" => self
                .playlist
                .as_ref()
                .map(|item| (item.name.clone(), item.detail.clone(), item.image.clone()))
                .unwrap_or_default(),
            "library" => match self.rows.get(self.selected) {
                Some(Row::Playlist(item)) => {
                    (item.name.clone(), item.detail.clone(), item.image.clone())
                }
                _ => (
                    "Your playlists".into(),
                    "Your music, ready for the sofa.".into(),
                    String::new(),
                ),
            },
            "devices" => (
                "Choose a player".into(),
                "Open Spotify on this PC or another device, then refresh this list.".into(),
                String::new(),
            ),
            _ => (
                "Make it yours".into(),
                "Connect your Spotify Premium account.".into(),
                String::new(),
            ),
        };
        ui.set_music_title(title.into());
        ui.set_music_detail(detail.into());
        if cover != self.cover_url {
            self.cover_url = cover;
            self.cover_at = Instant::now();
            self.cover_requested = false;
            ui.set_music_has_art(false);
            ui.set_music_artwork(slint::Image::default());
        }
        ui.set_music_track(if self.playback.name.is_empty() {
            "Nothing playing yet".into()
        } else {
            self.playback.name.as_str().into()
        });
        ui.set_music_artist(self.playback.artist.as_str().into());
        ui.set_music_playing(self.playback.playing);
        ui.set_music_device(if self.playback.device.is_empty() {
            "Select a player in Devices".into()
        } else {
            self.playback.device.as_str().into()
        });
        self.progress();
    }

    fn progress(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let elapsed = if self.playback.playing {
                self.playback_at.elapsed().as_millis() as u64
            } else {
                0
            };
            let position = (self.playback.position_ms + elapsed).min(self.playback.duration_ms);
            ui.set_music_progress(if self.playback.duration_ms > 0 {
                position as f32 / self.playback.duration_ms as f32
            } else {
                0.0
            });
            let seconds = position / 1000;
            let total = self.playback.duration_ms / 1000;
            ui.set_music_time(
                format!(
                    "{}:{:02} / {}:{:02}",
                    seconds / 60,
                    seconds % 60,
                    total / 60,
                    total % 60
                )
                .into(),
            );
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Initial(saved, warning) => {
                self.initialized = true;
                self.connected = saved.token.is_some();
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_music_client_id(saved.client_id.into());
                }
                if self.ui.upgrade().is_some_and(|ui| ui.get_music_visible()) && !self.demo {
                    self.open();
                }
                if let Some(warning) = warning {
                    self.status(&warning);
                }
            }
            Event::Connected(result, id) => {
                self.signing_in = false;
                match result {
                    Ok(()) => {
                        self.connected = true;
                        self.library.clear();
                        self.library_next = None;
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_music_client_id(id.into());
                        }
                        self.library();
                        self.poll();
                    }
                    Err(error) => {
                        self.sync();
                        self.status(&error);
                    }
                }
            }
            Event::Disconnected(result) => match result {
                Ok(()) => {
                    self.connected = false;
                    self.library.clear();
                    self.library_next = None;
                    self.playback = Playback::default();
                    self.setup();
                    self.status("Signed out. Your saved Client ID can be used to reconnect.");
                }
                Err(error) => self.status(&error),
            },
            Event::Library(generation, result) if generation == self.generation => {
                self.busy = false;
                match result {
                    Ok(page) => {
                        let old_len = self.library.len();
                        self.library.extend(page.entries);
                        self.library_next = page.next;
                        self.rows = self.library.iter().cloned().map(Row::Playlist).collect();
                        if let Some(next) = &self.library_next {
                            self.rows.push(Row::More(next.clone()));
                        }
                        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
                        self.sync();
                        self.status(if self.library.is_empty() {"No playlists found. Save a playlist in Spotify, then press R to refresh."} else if old_len > 0 {"More playlists loaded."} else {"Choose a playlist. Enter opens it."});
                    }
                    Err(error) => {
                        self.sync();
                        self.status(&error);
                    }
                }
            }
            Event::Tracks(generation, result) if generation == self.generation => {
                self.busy = false;
                match result {
                    Ok(page) => {
                        self.rows.retain(|row| !matches!(row, Row::More(_)));
                        self.rows.extend(page.entries.into_iter().map(Row::Track));
                        if let Some(next) = page.next {
                            self.rows.push(Row::More(next));
                        }
                        self.sync();
                        self.status("Choose a track, or play the whole playlist.");
                    }
                    Err(error) => {
                        self.sync();
                        self.status(&error);
                    }
                }
            }
            Event::Devices(generation, result) if generation == self.generation => {
                self.busy = false;
                match result {
                    Ok(entries) => {
                        self.rows = entries.into_iter().map(Row::Device).collect();
                        self.sync();
                        self.status(if self.rows.is_empty() {
                            "No players found. Open Spotify on your PC, then press R to refresh."
                        } else {
                            "Choose where music should play. Enter selects a device."
                        });
                    }
                    Err(error) => {
                        self.sync();
                        self.status(&error);
                    }
                }
            }
            Event::Playback(result) => {
                self.poll_pending = false;
                if let Ok(playback) = result
                    && self.ui.upgrade().is_some_and(|ui| ui.get_music_visible())
                {
                    self.playback = playback;
                    self.playback_at = Instant::now();
                    self.sync();
                }
                // Poll failures do not overwrite a foreground action's useful error message.
            }
            Event::Action(result) => match result {
                Ok(()) => {
                    self.status("Playback updated.");
                    self.poll();
                }
                Err(error) => self.status(&error),
            },
            Event::Art(url, Ok((bytes, width, height))) if url == self.cover_url => {
                if let Some(ui) = self.ui.upgrade() {
                    let buffer =
                        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&bytes, width, height);
                    ui.set_music_artwork(slint::Image::from_rgba8(buffer));
                    ui.set_music_has_art(true);
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            self.event(event);
        }
        let visible = self.ui.upgrade().is_some_and(|ui| ui.get_music_visible());
        if visible {
            self.progress();
            if self.initialized && !self.busy && self.last_poll.elapsed() > Duration::from_secs(5) {
                self.poll();
            }
            if !self.cover_requested
                && !self.cover_url.is_empty()
                && self.cover_at.elapsed() > Duration::from_millis(350)
                && !self.busy
            {
                self.cover_requested = true;
                self.send(Job::Art(self.cover_url.clone()));
            }
        }
    }
}

fn spotify_link(uri: &str) -> Option<String> {
    let mut parts = uri.split(':');
    if parts.next()? != "spotify" {
        return None;
    }
    let kind = parts.next()?;
    let id = parts.next()?;
    if !["playlist", "track", "album", "episode"].contains(&kind)
        || id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(format!("https://open.spotify.com/{kind}/{id}"))
}

fn demo_playlists() -> Vec<Entry> {
    [
        "Late-night favorites",
        "Slow Sunday",
        "On the road",
        "Focus time",
        "Weekend energy",
    ]
    .iter()
    .enumerate()
    .map(|(index, name)| Entry {
        id: format!("demo{index}"),
        name: (*name).into(),
        detail: "Sample playlist · SoffaShell demo".into(),
        uri: format!("spotify:playlist:demo{index}"),
        playable: true,
        ..Entry::default()
    })
    .collect()
}
fn demo_tracks() -> Vec<Entry> {
    [
        "After the lights",
        "Somewhere quiet",
        "One more evening",
        "Home again",
    ]
    .iter()
    .enumerate()
    .map(|(position, name)| Entry {
        name: (*name).into(),
        detail: "SoffaShell Demo · 3:30".into(),
        position,
        playable: true,
        ..Entry::default()
    })
    .collect()
}

pub struct Music {
    controller: Rc<RefCell<Controller>>,
    _timer: Timer,
}

impl Music {
    pub fn new(ui: &Shell, demo: bool, desktop: Rc<dyn Fn()>) -> Self {
        let (sender, jobs) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        if !demo {
            let cancelled = cancelled.clone();
            std::thread::spawn(move || worker(jobs, events, cancelled));
        }
        let controller = Rc::new(RefCell::new(Controller {
            desktop,
            ui: ui.as_weak(),
            sender,
            receiver,
            cancelled,
            demo,
            initialized: demo,
            connected: false,
            signing_in: false,
            busy: false,
            poll_pending: false,
            generation: 0,
            page: "setup",
            rows: vec![],
            selected: 0,
            menu: 0,
            focus: 1,
            playlist: None,
            library: vec![],
            library_next: None,
            library_selection: 0,
            playback: Playback::default(),
            last_poll: Instant::now() - Duration::from_secs(10),
            playback_at: Instant::now(),
            cover_url: String::new(),
            cover_at: Instant::now(),
            cover_requested: false,
        }));
        let control = controller.clone();
        ui.on_music_navigate(move |h, v| control.borrow_mut().navigate(h, v));
        let control = controller.clone();
        ui.on_music_activate(move || control.borrow_mut().activate());
        let control = controller.clone();
        ui.on_music_back(move || control.borrow_mut().back());
        let control = controller.clone();
        ui.on_music_action(move |action| {
            control.borrow_mut().action(&action);
        });
        let control = controller.clone();
        ui.on_music_choose(move |index| {
            let mut control = control.borrow_mut();
            control.focus = 1;
            control.selected = (index as usize).min(control.rows.len().saturating_sub(1));
            control.activate();
        });
        let control = controller.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            control.borrow_mut().tick()
        });
        Self {
            controller,
            _timer: timer,
        }
    }
    pub fn open(&self) {
        self.controller.borrow_mut().open();
    }
}

impl Drop for Music {
    fn drop(&mut self) {
        self.controller
            .borrow()
            .cancelled
            .store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{
        Key, Platform, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };

    struct TestPlatform(Rc<MinimalSoftwareWindow>);
    impl Platform for TestPlatform {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    fn press(ui: &Shell, key: Key) {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }

    fn render(window: &MinimalSoftwareWindow, name: &str) -> Vec<slint::Rgb8Pixel> {
        let size = window.size();
        let mut pixels = vec![slint::Rgb8Pixel::default(); (size.width * size.height) as usize];
        window.request_redraw();
        assert!(window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, size.width as usize);
        }));
        assert!(pixels.iter().any(|pixel| *pixel != pixels[0]));
        if let Some(directory) = std::env::var_os("SOFFASHELL_PREVIEW_DIR") {
            let path = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&path).unwrap();
            let bytes = pixels
                .iter()
                .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
                .collect::<Vec<_>>();
            image::save_buffer(
                path.join(format!("{name}.png")),
                &bytes,
                size.width,
                size.height,
                image::ColorType::Rgb8,
            )
            .unwrap();
        }
        pixels
    }

    #[test]
    fn keyboard_navigation_and_music_pages_render_without_a_desktop() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
        let ui = Shell::new().unwrap();
        ui.set_apps(
            Rc::new(VecModel::from(vec![crate::AppTile {
                name: "Spotify".into(),
                category: "LISTEN".into(),
                description: "Music".into(),
            }]))
            .into(),
        );
        let music = Music::new(&ui, true, Rc::new(|| {}));
        let weak = ui.as_weak();
        ui.on_back(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_music_back();
            }
        });
        ui.show().unwrap();
        window.set_size(slint::PhysicalSize::new(1280, 800));
        music.open();
        ui.invoke_focus_navigation();
        assert_eq!(ui.get_music_page(), "library");
        render(&window, "library");
        // A real downloaded cover must stay inside its panel, regardless of source resolution.
        let cover = [255u8, 0, 0, 255].repeat(600 * 600);
        ui.set_music_artwork(slint::Image::from_rgba8(
            SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&cover, 600, 600),
        ));
        ui.set_music_has_art(true);
        for (width, height) in [(1000, 720), (1280, 800), (1920, 1080)] {
            window.set_size(slint::PhysicalSize::new(width, height));
            let pixels = render(&window, &format!("library-artwork-{width}"));
            let red = pixels
                .iter()
                .enumerate()
                .filter(|(_, pixel)| pixel.r > 240 && pixel.g < 10 && pixel.b < 10)
                .map(|(index, _)| ((index as u32) % width, (index as u32) / width))
                .collect::<Vec<_>>();
            assert!(red.len() > 10000, "Artwork was not rendered");
            let side = (width as f32 * 0.23)
                .min(height as f32 * 0.38)
                .min(340.0)
                .ceil() as usize;
            assert!(
                red.len() <= side * side,
                "Artwork escaped its proportional panel at {width}x{height}"
            );
            assert!(
                red.iter()
                    .all(|(x, y)| *x < width * 3 / 5 && *y < height * 3 / 5),
                "Artwork overlapped the track list / player"
            );
        }
        window.set_size(slint::PhysicalSize::new(1280, 800));
        ui.set_music_has_art(false);
        press(&ui, Key::DownArrow);
        assert_eq!(ui.get_music_selected(), 1);
        press(&ui, Key::Return);
        assert_eq!(ui.get_music_page(), "tracks");
        render(&window, "tracks");
        press(&ui, Key::DownArrow);
        press(&ui, Key::Return);
        assert_eq!(ui.get_music_page(), "now-playing");
        assert!(ui.get_music_playing());
        render(&window, "now-playing");
        press(&ui, Key::Return);
        assert!(!ui.get_music_playing());
        press(&ui, Key::Escape);
        assert_eq!(ui.get_music_page(), "library");
        assert_eq!(ui.get_music_selected(), 1);
        press(&ui, Key::LeftArrow);
        assert_eq!(ui.get_music_focus(), 0);
        ui.invoke_music_action("devices".into());
        render(&window, "devices");
        ui.invoke_music_action("account".into());
        render(&window, "setup");
        press(&ui, Key::RightArrow);
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: "rnp123".into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: "rnp123".into(),
        });
        assert_eq!(ui.get_music_client_id(), "rnp123");
        // Returning from setup restores navigation focus, rather than quitting the shell.
        press(&ui, Key::Escape);
        assert!(ui.get_music_visible());
        // Delayed results for an old page must not overwrite the new page.
        let old_generation = music.controller.borrow().generation;
        ui.invoke_music_action("devices".into());
        music
            .controller
            .borrow_mut()
            .event(Event::Library(old_generation, Ok(Page::default())));
        assert_eq!(ui.get_music_page(), "devices");
        // A cover response for an earlier selection cannot replace the current artwork.
        music.controller.borrow_mut().event(Event::Art(
            "https://i.scdn.co/old".into(),
            Ok((vec![255; 4], 1, 1)),
        ));
        assert!(!ui.get_music_has_art());
        window.set_size(slint::PhysicalSize::new(1000, 720));
        render(&window, "devices-small");
        // Ctrl+Meta+X exits the music interface and clears playback, even while editing.
        // Demo mode deliberately doesn't signal any real processes.
        ui.invoke_music_action("account".into());
        press(&ui, Key::RightArrow);
        music.controller.borrow_mut().playback.playing = true;
        music.controller.borrow_mut().sync();
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Meta.into(),
        });
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text: "x".into() });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Meta.into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        assert!(!ui.get_music_visible());
        assert!(!ui.get_music_playing());
        assert_eq!(ui.get_music_client_id(), "rnp123");
        ui.hide().unwrap();
    }
}
