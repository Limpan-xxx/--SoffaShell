//! Close only the current user's local Spotify executables, never other media players.

fn is_spotify_executable(path: &std::path::Path) -> bool {
    matches!(
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.trim_end_matches(" (deleted)")),
        Some("spotify" | "spotify-launcher")
    )
}

#[cfg(target_os = "linux")]
pub fn quit_spotify() -> Result<(), String> {
    use std::{
        fs,
        os::unix::fs::MetadataExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let uid = fs::metadata("/proc/self")
        .map_err(|_| "Could not identify the current user")?
        .uid();
    let mut processes = Vec::new();
    for entry in fs::read_dir("/proc")
        .map_err(|_| "Could not inspect running applications")?
        .flatten()
    {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let path = entry.path();
        if fs::metadata(&path).is_ok_and(|metadata| metadata.uid() == uid)
            && let Ok(executable) = fs::read_link(path.join("exe"))
            && is_spotify_executable(&executable)
        {
            processes.push((pid, executable));
        }
    }
    // Remember executable identity, so a recycled PID doesn't target a different application.
    let still_running = |pid: u32, executable: &std::path::Path| {
        let path = format!("/proc/{pid}");
        fs::metadata(&path).is_ok_and(|metadata| metadata.uid() == uid)
            && fs::read_link(format!("{path}/exe")).is_ok_and(|current| current == executable)
    };
    let signal = |name: &str, pid: u32| -> Result<(), String> {
        Command::new("kill")
            .args(["--signal", name, "--", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "Could not run kill to close Spotify".to_owned())?;
        Ok(())
    };
    // Ask every instance to exit normally first. A stuck instance is stopped after two seconds.
    for (pid, executable) in &processes {
        if still_running(*pid, executable) {
            signal("TERM", *pid)?;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && processes.iter().any(|(pid, exe)| still_running(*pid, exe)) {
        std::thread::sleep(Duration::from_millis(50));
    }
    for (pid, executable) in &processes {
        if still_running(*pid, executable) {
            signal("KILL", *pid)?;
        }
    }
    // Allow the kernel to finish terminating before reporting completion.
    for _ in 0..20 {
        if processes.iter().all(|(pid, exe)| !still_running(*pid, exe)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("Some Spotify processes could not be closed. Check their permissions.".into())
}

#[cfg(not(target_os = "linux"))]
pub fn quit_spotify() -> Result<(), String> {
    Err("Closing local Spotify processes is currently supported on Linux only.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quit_matches_only_spotify_executables_including_updated_binaries() {
        for name in [
            "/usr/bin/spotify",
            "/usr/bin/spotify-launcher",
            "/opt/spotify/spotify (deleted)",
        ] {
            assert!(is_spotify_executable(std::path::Path::new(name)));
        }
        for name in [
            "/usr/bin/soffashell",
            "/usr/bin/spotify-helper",
            "/usr/bin/spotifyd",
            "/usr/bin/firefox",
        ] {
            assert!(!is_spotify_executable(std::path::Path::new(name)));
        }
    }
}
