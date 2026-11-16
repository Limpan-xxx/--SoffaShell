mod config;
mod icons;
mod music;
mod navigation;
mod processes;
mod spotify;

use slint::ComponentHandle;
use std::{error::Error, path::PathBuf, process::Command, rc::Rc};

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    let mut config_path = None;
    let mut fullscreen = true;
    let mut demo_spotify = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--fullscreen") => fullscreen = true,
            Some("--windowed") => fullscreen = false,
            Some("--demo-spotify") => demo_spotify = true,
            Some("--config") => {
                config_path = Some(PathBuf::from(args.next().ok_or("--config needs a path")?));
            }
            Some("--help" | "-h") => {
                println!(
                    "SoffaShell\n\nUsage: soffashell [--windowed | --fullscreen] [--config PATH] [--demo-spotify]\n\nStarts fullscreen by default. Use --windowed for development.\n\nArrows: select\nEnter: open / play\nEscape / Backspace: back; leave fullscreen or quit from home\nF11: fullscreen\nMusic: Space plays/pauses, N/P skips, R refreshes\n--demo-spotify: preview the music interface with sample data (no account or audio)"
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {}", arg.to_string_lossy()).into()),
        }
    }

    let config = Rc::new(config::Config::load(config_path.as_deref())?);
    let ui = Shell::new()?;
    let tiles = config.apps.iter().map(icons::tile).collect::<Vec<_>>();
    ui.set_apps(Rc::new(slint::VecModel::from(tiles)).into());
    ui.window().set_fullscreen(fullscreen);

    let weak = ui.as_weak();
    let desktop_config = config.clone();
    let open_spotify = Rc::new(move || {
        if let Some(ui) = weak.upgrade()
            && let Some(app) = desktop_config.apps.iter().find(|app| is_spotify(app))
        {
            launch_external(
                &ui,
                app.name.clone(),
                app.command.clone(),
                app.args.clone(),
                false,
            );
        }
    });
    let music = Rc::new(music::Music::new(&ui, demo_spotify, open_spotify));
    if demo_spotify {
        music.open();
    }

    let weak = ui.as_weak();
    let count = config.apps.len();
    ui.on_navigate(move |direction| {
        if let Some(ui) = weak.upgrade() {
            let next = navigation::next_selection(ui.get_selected() as usize, direction, count);
            ui.set_selected(next as i32);
            ui.set_status("Ready when you are.".into());
        }
    });

    let weak = ui.as_weak();
    ui.on_launch(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_launching() {
            return;
        }
        let Some(app) = config.apps.get(ui.get_selected() as usize) else {
            return;
        };
        if is_spotify(app) {
            music.open();
            if !demo_spotify {
                launch_external(
                    &ui,
                    app.name.clone(),
                    app.command.clone(),
                    app.args.clone(),
                    true,
                );
            }
            return;
        }
        let name = app.name.clone();
        let command = app.command.clone();
        let arguments = app.args.clone();
        launch_external(&ui, name, command, arguments, false);
    });

    let weak = ui.as_weak();
    ui.on_toggle_fullscreen(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().set_fullscreen(!ui.window().is_fullscreen());
        }
    });

    let weak = ui.as_weak();
    ui.on_back(move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_music_visible() {
                ui.invoke_music_back();
            } else if ui.window().is_fullscreen() {
                ui.window().set_fullscreen(false);
            } else {
                let _ = slint::quit_event_loop();
            }
        }
    });
    ui.run()?;
    Ok(())
}

fn is_spotify(app: &config::App) -> bool {
    app.interface.as_deref() == Some("spotify")
        || (app.interface.is_none() && app.name.eq_ignore_ascii_case("spotify"))
}

fn launch_external(
    ui: &Shell,
    name: String,
    command: String,
    arguments: Vec<String>,
    background_spotify: bool,
) {
    if ui.get_launching() {
        return;
    }
    ui.set_launching(true);
    ui.set_status(format!("Opening {name}…").into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let guard = if background_spotify {
            Some(spotify::window::StartupGuard::start())
        } else {
            None
        };
        let warning = guard
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .cloned();
        let result = Command::new(&command).args(&arguments).spawn();
        let message = match &result {
            Ok(_) if background_spotify => match warning {
                Some(error) => format!("Started {name}. Could not keep it minimized: {error}"),
                None => format!("Started {name} in the background. Choose a player in Devices."),
            },
            Ok(_) => format!("Started {name}. Use Alt+Tab to return here."),
            Err(error) => format!("Could not start {name} ({command}): {error}"),
        };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_launching(false);
            ui.set_status(message.clone().into());
            if ui.get_music_visible() {
                ui.set_music_status(message.into());
            }
        });
        if result.is_ok()
            && let Some(Ok(guard)) = guard
        {
            guard.keep_during_startup();
        }
        if let Ok(mut child) = result {
            match child.wait() {
                Ok(status) if !status.success() => eprintln!("{name} exited with {status}"),
                Err(error) => eprintln!("Could not wait for {name}: {error}"),
                _ => {}
            }
        }
    });
}
