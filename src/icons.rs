//! Resolve installed desktop icons; no logo downloads or bundled app-icon copies.
use crate::{AppTile, config::App};
use std::path::{Path, PathBuf};

fn desktop_icon(text: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        }
        if in_entry && let Some(icon) = line.strip_prefix("Icon=") {
            return Some(icon.trim().to_owned());
        }
    }
    None
}

fn data_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(path.into());
    } else if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share"));
    }
    if let Some(paths) = std::env::var_os("XDG_DATA_DIRS") {
        roots.extend(std::env::split_paths(&paths));
    }
    roots.extend(
        [
            "/usr/local/share",
            "/usr/share",
            "/var/lib/flatpak/exports/share",
        ]
        .map(PathBuf::from),
    );
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/flatpak/exports/share"));
    }
    roots
}

fn find(names: &[String], roots: &[PathBuf]) -> Option<PathBuf> {
    // Desktop entries may use an absolute icon filename.
    for name in names {
        let path = Path::new(name);
        if path.is_absolute() && path.is_file() {
            return Some(path.into());
        }
    }
    let mut themes = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        themes.push(PathBuf::from(home).join(".icons/hicolor"));
    }
    for root in roots {
        themes.push(root.join("icons/hicolor"));
    }
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(root.join("icons")) {
            let mut others = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            others.sort();
            themes.extend(others);
        }
    }
    // Rank image quality before theme / alias order. A tiny icon in the first
    // theme must not hide a vector (or a larger bitmap) in another installed theme.
    for extension in ["svg", "png", "xpm"] {
        for size in [
            "scalable/apps",
            "apps/scalable",
            "512x512/apps",
            "apps/512",
            "256x256/apps",
            "apps/256",
            "128x128/apps",
            "apps/128",
            "64x64/apps",
            "apps/64",
            "48x48/apps",
            "apps/48",
            "32x32/apps",
            "apps/32",
            "apps/symbolic",
        ] {
            for name in names {
                let filename = if matches!(
                    Path::new(name).extension().and_then(|value| value.to_str()),
                    Some("png" | "svg" | "xpm")
                ) {
                    if Path::new(name).extension().and_then(|value| value.to_str())
                        != Some(extension)
                    {
                        continue;
                    }
                    name.clone()
                } else {
                    format!("{name}.{extension}")
                };
                for theme in &themes {
                    let file = theme.join(size).join(&filename);
                    if file.is_file() {
                        return Some(file);
                    }
                }
            }
        }
        for name in names {
            for root in roots {
                let file = root.join("pixmaps").join(format!("{name}.{extension}"));
                if file.is_file() {
                    return Some(file);
                }
            }
        }
    }
    None
}

pub fn tile(app: &App) -> AppTile {
    let roots = data_roots();
    let mut names = Vec::new();
    if let Some(icon) = &app.icon {
        names.push(icon.clone());
    }
    let command = Path::new(&app.command)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&app.command);
    for id in [
        command.to_owned(),
        app.name.to_lowercase(),
        format!("com.{}.{}", app.name.to_lowercase(), app.name),
    ] {
        for root in &roots {
            if let Ok(text) =
                std::fs::read_to_string(root.join("applications").join(format!("{id}.desktop")))
                && let Some(icon) = desktop_icon(&text)
            {
                names.push(icon);
            }
        }
    }
    names.extend([command.to_owned(), app.name.to_lowercase()]);
    if app.name.eq_ignore_ascii_case("spotify") {
        names.extend(["spotify", "spotify-client", "com.spotify.Client"].map(str::to_owned));
    }
    if app.name.eq_ignore_ascii_case("stremio") {
        names.push("com.stremio.Stremio".into());
    }
    let path = app
        .icon
        .as_ref()
        .and_then(|icon| find(std::slice::from_ref(icon), &roots))
        .or_else(|| find(&names, &roots));
    let icon = path.and_then(|path| slint::Image::load_from_path(&path).ok());
    AppTile {
        name: app.name.as_str().into(),
        category: app.category.as_str().into(),
        description: app.description.as_str().into(),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_main_desktop_icon_without_using_action_icons() {
        assert_eq!(
            desktop_icon(
                "[Desktop Entry]\nName=App\nIcon=steam\n[Desktop Action test]\nIcon=other"
            ),
            Some("steam".into())
        );
        assert_eq!(desktop_icon("[Desktop Action test]\nIcon=other"), None);
    }

    #[test]
    fn prefers_vectors_across_aliases_then_largest_bitmaps() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("soffashell-icons-{stamp}"));
        let small = root.join("icons/hicolor/256x256/apps/soffashell-test-raster.png");
        let large = root.join("icons/hicolor/512x512/apps/soffashell-test-raster.png");
        let vector = root.join("icons/another/scalable/apps/soffashell-test-vector.svg");
        for file in [&small, &large, &vector] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, []).unwrap();
        }
        let roots = vec![root.clone()];
        let names = vec![
            "soffashell-test-raster".into(),
            "soffashell-test-vector".into(),
        ];
        assert_eq!(find(&names, &roots), Some(vector.clone()));
        std::fs::remove_file(vector).unwrap();
        assert_eq!(find(&names, &roots), Some(large));
        assert_eq!(find(&[small.to_str().unwrap().into()], &roots), Some(small));
        std::fs::remove_dir_all(root).unwrap();
    }
}
