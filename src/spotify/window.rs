//! A temporary KDE startup guard. Desktop Spotify launches retain their normal behavior.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zbus::blocking::{Connection, Proxy, connection::Builder};

pub struct StartupGuard {
    connection: Connection,
    plugin: String,
    file: PathBuf,
}

impl StartupGuard {
    pub fn start() -> Result<Self, String> {
        if !std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("KDE"))
        {
            return Err("Background Spotify startup currently requires KDE Plasma 6".into());
        }
        let connection = Builder::session()
            .map_err(|e| e.to_string())?
            .method_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| e.to_string())?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pid = std::process::id();
        let plugin = format!("soffashell-spotify-start-{pid}-{timestamp}");
        let file = std::env::temp_dir().join(format!("{plugin}.js"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(&file).map_err(|e| e.to_string())?;
        let guard = Self {
            connection,
            plugin,
            file,
        };
        let settings = serde_json::json!({"shellPid":pid,"plugin":guard.plugin});
        writeln!(
            output,
            "const settings = {settings};\n{}",
            include_str!("../../desktop/spotify-startup.js")
        )
        .map_err(|e| e.to_string())?;
        drop(output);
        let proxy = guard.scripting().map_err(|e| e.to_string())?;
        let id: i32 = proxy
            .call(
                "loadScript",
                &(
                    guard.file.to_str().ok_or("Invalid startup script path")?,
                    guard.plugin.as_str(),
                ),
            )
            .map_err(|e| e.to_string())?;
        if id < 0 {
            return Err("KWin refused the Spotify startup script".into());
        }
        let path = format!("/Scripting/Script{id}");
        let script = Proxy::new(
            &guard.connection,
            "org.kde.KWin",
            path.as_str(),
            "org.kde.kwin.Script",
        )
        .map_err(|e| e.to_string())?;
        let _: () = script.call("run", &()).map_err(|e| e.to_string())?;
        drop(script);
        drop(proxy);
        Ok(guard)
    }

    fn scripting(&self) -> zbus::Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
        )
    }

    pub fn keep_during_startup(self) {
        // The script expires independently; keep its source and D-Bus connection alive
        // while spotify-launcher checks for updates and the desktop client initializes.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            drop(self);
        });
    }
}

impl Drop for StartupGuard {
    fn drop(&mut self) {
        if let Ok(proxy) = self.scripting() {
            let _: zbus::Result<bool> = proxy.call("unloadScript", &(self.plugin.as_str(),));
        }
        let _ = fs::remove_file(&self.file);
    }
}
