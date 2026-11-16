# SoffaShell

A Rust + Slint launcher for a computer connected to your TV, with an integrated
Spotify interface designed for keyboard / remote navigation.

## Run

Requires Rust 1.92+, a graphical desktop session, and network access for Cargo's
first dependency download. Uses winit and software rendering; Qt is not required.

```bash
cargo run
cargo run -- --windowed
```

| Key | Action |
| --- | --- |
| Left (or Up) | Previous app (wraps around) |
| Right (or Down) | Next app (wraps around) |
| Enter | Launch selected app |
| F11 | Toggle fullscreen |
| Escape / Backspace | Leave fullscreen; quit if already windowed |
| Alt+Tab | Switch between the launcher and desktop apps |

Clicking a tile also launches it. Starts fullscreen by default; use `--windowed`
for development. F11 toggles fullscreen while running.

The home screen displays a horizontal row of app logos. Hovering selects a tile;
the selected tile grows while one focus outline slides between cards over 260 ms.
Arrow keys use the same selection,
and longer app lists scroll horizontally to keep the selected tile visible.
Logos are loaded from installed desktop entries and host icon themes / pixmaps,
preferring scalable SVGs and then large PNGs (including 512×512) for TV displays;
SoffaShell does not download or bundle copies of the launcher icons. If an icon
is missing, the app name is shown. An app entry can optionally set `icon` to an
installed icon name or an absolute path to an existing icon file.

## Unified Remote keyboard input

With your Unified Remote server running, send keyboard keys to the focused
SoffaShell window: arrows navigate, Enter activates, Escape or Backspace goes
back, and F11 toggles fullscreen. Held arrows keep moving; held activation,
back, and fullscreen keys act only once per press.

Navigation also works when a music button has focus. In the Spotify Client ID
field, arrows and Backspace edit text as usual; Escape leaves the page and F11
still toggles fullscreen. Modified keys are left for widget / desktop shortcuts.
These handlers consume normal keyboard events; no separate remote protocol or
network listener is required in SoffaShell. Once another app has focus, keyboard
input goes to that app; use Alt+Tab to return.

## Spotify TV interface

Selecting Spotify opens SoffaShell's own music screen and starts the configured
Spotify executable (`spotify-launcher` by default) to provide audio playback.
On KDE Plasma 6, Spotify's desktop window is minimized during startup so SoffaShell
keeps focus. Browse your playlists,
open a playlist's tracks, choose a Spotify Connect device, and use the Now Playing
view with album artwork, progress, play/pause, and previous/next controls.
Spotify's desktop app or another Spotify Connect player handles audio playback;
SoffaShell is the browsing and control interface, not a streaming player.

Background startup uses a temporary KWin script armed before the player starts.
It handles existing and newly created Spotify windows, then unloads two seconds
after finding a window (or after 30 seconds if none appears). It adds no permanent
window rules. The Desktop app menu action still opens Spotify normally. If KDE
integration is unavailable, the status bar explains the failure and Spotify starts
normally; use Alt+Tab to return in that case.

Try the interface before creating a Developer app:

```bash
cargo run -- --demo-spotify
```

Demo mode is labelled on screen and uses invented playlists and tracks. It makes
no Spotify API calls and does not play audio or launch the desktop Spotify app.

### Connect your account

Requires Spotify Premium, `curl`, and `openssl` on `PATH`. The browser is used only
for account authorization; browsing and playback controls remain in Slint.

1. Open the [Spotify Developer dashboard](https://developer.spotify.com/dashboard)
   and create an app using your Premium account. Select Web API if prompted.
2. Register this exact Redirect URI: **`http://127.0.0.1:8888/callback`**.
   Spotify requires a loopback IP address rather than `localhost`.
3. Copy the app's **Client ID**. You do not need a client secret.
4. In SoffaShell, select Spotify, paste the Client ID, and select Connect Spotify.
   Finish authorization in your browser, then return to SoffaShell. Escape cancels
   a pending sign-in; sign-in also expires after three minutes.
5. Spotify starts automatically when you open the Spotify tile. Choose
   **Devices**, press **R** to refresh, then select the player you want.
6. Choose a playlist and select **Play playlist** or a track.

If no player appears, open Spotify and play/pause something once, then refresh
Devices. The **Desktop app** menu entry opens the configured Spotify executable;
use Alt+Tab to return. If a development-mode app returns 403, check that the
account you're signing in with is permitted under the app's Users and Access.

Spotify currently permits track-list reads only for playlists you own or
collaborate on. Other playlists can still appear in your library; their page
keeps the Play playlist action and explains the API restriction. Browse lists
using Load more. This first version covers playlists and playback, not Spotify's
complete search, recommendations, or album library.

| Music key | Action |
| --- | --- |
| Up / Down | Select a menu item, playlist, track, or playback control |
| Left / Right | Switch between the sidebar and content |
| Enter | Open / play / activate selected item |
| Escape / Backspace | Back to playlists, then launcher; cancels pending sign-in |
| Ctrl+Windows+X | Quit local Spotify instances and Spotify Launcher, then return home |
| Space | Play / pause |
| N / P | Next / previous track |
| R | Refresh the current list / playback state |

On the setup page, Right focuses the Client ID field; paste with Ctrl+V and press
Enter to connect. Typing into the field does not trigger the playback shortcuts.
Ctrl+Windows+X also works while the Client ID field has focus. This is an app
shortcut: SoffaShell must have focus; if desktop Spotify takes focus, use Alt+Tab
to return first. On Linux it closes every `spotify` and `spotify-launcher`
executable owned by your user, including instances started outside SoffaShell.
Instances get a normal termination request first; stuck processes are force-stopped
after two seconds. Other Spotify Connect devices are unaffected. Demo mode closes
only the demo interface. Escape and Back home remain navigation actions.

Sign-in uses Authorization Code with PKCE and validates the callback state. The
Client ID and tokens are saved locally in
`~/.local/state/soffashell/spotify.json`, outside the repository and MEGA folder,
with owner-only permissions on Linux. Tokens refresh automatically. Sign out
forgets the local tokens while retaining the Client ID. You can also provide the
public Client ID using `SOFFASHELL_SPOTIFY_CLIENT_ID`.

Network work runs on a background thread, with request timeouts and rate-limit
backoff. Small artwork previews are cached in memory only. Credentials are passed
to `curl` through stdin rather than command-line arguments. No Spotify credentials
belong in `config/apps.toml` or Git.

References: [PKCE](https://developer.spotify.com/documentation/web-api/tutorials/code-pkce-flow),
[redirect URIs](https://developer.spotify.com/documentation/web-api/concepts/redirect_uri),
[playlist item access](https://developer.spotify.com/documentation/web-api/reference/get-playlists-items).

## Configure apps

Defaults in `config/apps.toml` launch Linux desktop installations of Steam (Big
Picture) and Stremio. Spotify opens the integrated interface; its Desktop app
action uses `spotify-launcher`. Install applications separately. Missing
executables produce an on-screen error.

For local changes, copy the defaults to the gitignored configuration:

```bash
cp config/apps.toml config/local.toml
cargo run -- --config config/local.toml
```

For Flatpak installations, change the command and arguments, for example:

```toml
[[apps]]
name = "Spotify"
interface = "spotify"
category = "LISTEN"
description = "Give the room a soundtrack."
command = "flatpak"
args = ["run", "com.spotify.Client"]
```

Commands execute directly without a shell. Use an absolute executable path or a
command on `PATH`; put each argument in its own array entry. Shell expansion
(`~`, `$HOME`) and shell operators are not supported. Without `--config`, defaults
are embedded at build time. An explicit configuration replaces the entire app list.

`interface = "spotify"` selects the built-in music interface. Old configurations
with an app named Spotify also open it automatically. Set `interface = "desktop"`
to keep a tile as a plain external-app launcher instead.

## Project layout

- `ui/shell.slint`: TV layout and keyboard-to-action mapping.
- `src/main.rs`: CLI, UI callbacks, and background app launching.
- `src/config.rs`: TOML app configuration and validation.
- `ui/music.slint`: large music controls, sidebar, artwork, and setup screen.
- `src/music.rs`: music navigation, view state, background jobs, and demo mode.
- `src/spotify/`: PKCE sign-in, token storage, API client, and response parsing.
- `src/navigation.rs`: selection logic for keyboard and future remote inputs.
- `config/apps.toml`: starter app definitions.

## Development

Development builds omit debug symbols and incremental compilation caches to save
disk space. Recompilation after edits can take longer. Cargo still caches compiled
dependencies. To enable debugging temporarily, run
`CARGO_PROFILE_DEV_DEBUG=1 cargo run`.

For a release executable with symbols stripped, use `cargo build --release`.
`cargo clean` removes generated build files when you need to reclaim disk space.

On this computer, the gitignored `.cargo/config.toml` directs builds to
`~/.cache/soffashell/target`, outside MEGA. On another machine, choose an external
build directory with Cargo's `build.target-dir` setting or `CARGO_TARGET_DIR`.
Cloud sync needs its own exclusions: `.gitignore` does not prevent MEGA uploads.

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
node desktop/spotify-startup.test.cjs
```

The tests include PKCE vectors, callback validation, pagination / rate-limit
handling, and headless rendering with actual Slint arrow-key events. Set
`SOFFASHELL_PREVIEW_DIR=/tmp/soffashell-preview` when running `cargo test` to save
screenshots outside the repo. These tests don't access a real Spotify account.

This is a launcher skeleton. Keyboard events belong to the focused app. Controls
inside Steam and Stremio, a global Home shortcut, direct remote protocol support, automatic
focus restoration, and boot-to-launcher integration are future work. A successful
launch means the process started; apps may exit or hand off to an existing process.
Nonzero exit statuses are logged to the terminal.
