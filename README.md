# SoffaShell

A Rust + Slint launcher for a computer connected to your TV.

## Run

Requires Rust 1.92+, a graphical desktop session, and network access for Cargo's
first dependency download. Uses winit and software rendering; Qt is not required.

```bash
cargo run
cargo run -- --fullscreen
```

| Key | Action |
| --- | --- |
| Up / Left | Previous app (wraps around) |
| Down / Right | Next app (wraps around) |
| Enter | Launch selected app |
| F11 | Toggle fullscreen |
| Escape | Leave fullscreen; quit if already windowed |
| Alt+Tab | Switch between the launcher and desktop apps |

Clicking a tile also launches it. Starts windowed for development.

## Configure apps

Defaults in `config/apps.toml` launch Linux desktop installations of Steam (Big
Picture), Stremio, and Spotify. Install applications separately. Missing
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
category = "LISTEN"
description = "Give the room a soundtrack."
command = "flatpak"
args = ["run", "com.spotify.Client"]
```

Commands execute directly without a shell. Use an absolute executable path or a
command on `PATH`; put each argument in its own array entry. Shell expansion
(`~`, `$HOME`) and shell operators are not supported. Without `--config`, defaults
are embedded at build time. An explicit configuration replaces the entire app list.

## Project layout

- `ui/shell.slint`: TV layout and keyboard-to-action mapping.
- `src/main.rs`: CLI, UI callbacks, and background app launching.
- `src/config.rs`: TOML app configuration and validation.
- `src/navigation.rs`: selection logic for keyboard and future remote inputs.
- `config/apps.toml`: starter app definitions.

## Development

Development builds omit debug symbols and incremental compilation caches to save
disk space. Recompilation after edits can take longer. Cargo still caches compiled
dependencies. To enable debugging temporarily, run
`CARGO_PROFILE_DEV_DEBUG=1 cargo run`.

For a release executable with symbols stripped, use `cargo build --release`.
`cargo clean` removes generated build files when you need to reclaim disk space.

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

This is a launcher skeleton. Keyboard events belong to the focused app. Controls
inside Steam, Stremio, and Spotify, a global Home shortcut, remote input, automatic
focus restoration, and boot-to-launcher integration are future work. A successful
launch means the process started; apps may exit or hand off to an existing process.
Nonzero exit statuses are logged to the terminal.
