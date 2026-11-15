mod config;
mod navigation;

use slint::ComponentHandle;
use std::{error::Error, path::PathBuf, process::Command, rc::Rc};

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    let mut config_path = None;
    let mut fullscreen = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--fullscreen") => fullscreen = true,
            Some("--config") => {
                config_path = Some(PathBuf::from(args.next().ok_or("--config needs a path")?));
            }
            Some("--help" | "-h") => {
                println!(
                    "SoffaShell\n\nUsage: soffashell [--fullscreen] [--config PATH]\n\nArrows: select app\nEnter: launch\nF11: toggle fullscreen\nEscape: leave fullscreen, or quit when windowed"
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {}", arg.to_string_lossy()).into()),
        }
    }

    let config = Rc::new(config::Config::load(config_path.as_deref())?);
    let ui = Shell::new()?;
    let tiles = config
        .apps
        .iter()
        .map(|app| AppTile {
            name: app.name.as_str().into(),
            category: app.category.as_str().into(),
            description: app.description.as_str().into(),
        })
        .collect::<Vec<_>>();
    ui.set_apps(Rc::new(slint::VecModel::from(tiles)).into());
    ui.window().set_fullscreen(fullscreen);

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
        let name = app.name.clone();
        let command = app.command.clone();
        let arguments = app.args.clone();
        ui.set_launching(true);
        ui.set_status(format!("Opening {name}…").into());
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = Command::new(&command).args(&arguments).spawn();
            let message = match &result {
                Ok(_) => format!("Started {name}. Use Alt+Tab to return here."),
                Err(error) => format!("Could not start {name} ({command}): {error}"),
            };
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_launching(false);
                ui.set_status(message.into());
            });
            // Reap children without blocking the UI. Apps may hand off to another process.
            if let Ok(mut child) = result {
                match child.wait() {
                    Ok(status) if !status.success() => eprintln!("{name} exited with {status}"),
                    Err(error) => eprintln!("Could not wait for {name}: {error}"),
                    _ => {}
                }
            }
        });
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
            if ui.window().is_fullscreen() {
                ui.window().set_fullscreen(false);
            } else {
                let _ = slint::quit_event_loop();
            }
        }
    });
    ui.run()?;
    Ok(())
}
