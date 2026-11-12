mod collision;
#[cfg(windows)]
mod frontend;
mod game;
#[cfg(windows)]
mod hud;
mod hud_art;
mod light;
mod map;
mod movement;
mod mover;
mod music;
mod package;
#[cfg(windows)]
mod present;
mod props;
mod render;
mod script;
#[cfg(windows)]
mod settings;
mod sound;
mod teleport;
mod texture;
#[cfg(windows)]
mod window;

use std::{env, fs, io, path::Path, process::ExitCode};

use map::Map;
use package::Package;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1).peekable();
    #[cfg(windows)]
    let obs = args.next_if(|arg| arg == "--obs").is_some();
    if args.peek().is_none() {
        #[cfg(windows)]
        {
            let exe = env::current_exe()?;
            let demo = exe.with_file_name("Maps").join("DM-Deck16][.unr");
            if demo.is_file() {
                let bytes = fs::read(&demo)?;
                let package = Package::read(&bytes)?;
                let mut map = Map::load(&package, Some(&demo))?;
                let start = 6.min(map.starts.len() - 1);
                return Ok(window::show(
                    &mut map,
                    start,
                    &window_title(&demo),
                    obs,
                    &demo,
                    true,
                )?);
            }
        }
    }
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };
    if command == "--help" || command == "-h" {
        print_usage();
        return Ok(());
    }
    if !matches!(
        command.to_str(),
        Some("inspect" | "map-info" | "view" | "play" | "capture" | "capture-practice")
    ) {
        return Err(io::Error::other("Unknown command. Run with --help.").into());
    }
    let path = args
        .next()
        .ok_or_else(|| io::Error::other("Missing UE1 package path"))?;
    let output = if command == "capture" || command == "capture-practice" {
        Some(
            args.next()
                .ok_or_else(|| io::Error::other("Missing output BMP path"))?,
        )
    } else {
        None
    };
    let start = if command == "view"
        || command == "play"
        || command == "capture"
        || command == "capture-practice"
    {
        args.next()
            .map(|value| {
                value
                    .to_str()
                    .and_then(|value| value.parse::<usize>().ok())
                    .and_then(|index| index.checked_sub(1))
                    .ok_or_else(|| io::Error::other("Player start must be a positive integer"))
            })
            .transpose()?
            .unwrap_or(0)
    } else {
        0
    };
    if args.next().is_some() {
        return Err(io::Error::other("Unexpected argument").into());
    }
    let path = Path::new(&path);
    let bytes = fs::read(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    let package = Package::read(&bytes)?;
    println!(
        "{} — UE1 package v{} (licensee {})",
        path.display(),
        package.version,
        package.licensee_version
    );
    println!(
        "{} names, {} imports, {} exports",
        package.names.len(),
        package.imports.len(),
        package.exports.len()
    );
    for import in &package.imports {
        if import.outer == 0 && package.name(import.class)? == "Package" {
            println!("  package {}", package.name(import.name)?);
        }
    }
    if command != "inspect" {
        let mut map = Map::load(&package, (command != "map-info").then_some(path))?;
        println!(
            "{} visible BSP faces, {} player starts, {} moving brushes",
            map.faces.len(),
            map.starts.len(),
            map.movers.len()
        );
        if command != "map-info" {
            println!(
                "{} faces use original textures",
                map.faces
                    .iter()
                    .filter(|face| face.texture.is_some())
                    .count()
            );
        }
        if command != "map-info" {
            println!(
                "{} ambient emitters, {} lit faces",
                map.ambient.len(),
                map.faces
                    .iter()
                    .filter(|face| face.lightmap.is_some())
                    .count()
            );
        }
        if command != "map-info" {
            println!(
                "{} original mesh actors, {} teleporter links",
                map.props.len(),
                crate::teleport::link_count(&map.teleporters)
            );
        }
        let spawn = *map.starts.get(start).ok_or_else(|| {
            io::Error::other(format!(
                "Player start must be between 1 and {}",
                map.starts.len()
            ))
        })?;
        if let Some(output) = output {
            let mut frame = render::Frame::new();
            let mut camera = render::Camera::from_start(spawn);
            if command == "capture-practice" {
                let walker = collision::Walker::spawn(&map.collision, spawn.position);
                camera.position = walker.eye(map.collision.profile, camera.right());
                let mut practice = game::Practice::new(&map);
                practice.update(
                    &mut map,
                    &camera,
                    1.0,
                    game::Input {
                        primary: false,
                        alternate: false,
                        slot: None,
                        cycle: 0,
                    },
                    false,
                    None,
                );
                frame.draw_practice(&map, &camera, 0.0, &practice);
            } else {
                frame.draw(&map, &camera, 0.0);
            }
            frame.save_bmp(Path::new(&output))?;
        } else if command == "view" || command == "play" {
            #[cfg(windows)]
            window::show(
                &mut map,
                start,
                &window_title(path),
                obs,
                path,
                command == "play",
            )?;
            #[cfg(not(windows))]
            return Err(io::Error::other("Interactive view currently requires Windows").into());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn window_title(path: &Path) -> String {
    format!(
        "UT99 Rust Solo | {}",
        path.file_stem().unwrap_or_default().to_string_lossy()
    )
}

fn print_usage() {
    println!("ut99-rust inspect <package>");
    println!("ut99-rust map-info <map.unr>");
    println!("ut99-rust [--obs] view <map.unr> [player-start]");
    println!("ut99-rust [--obs] play <map.unr> [player-start]");
    println!("ut99-rust capture <map.unr> <output.bmp> [player-start]");
    println!("ut99-rust capture-practice <map.unr> <output.bmp> [player-start]");
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ut99-rust: {error}");
            ExitCode::FAILURE
        }
    }
}
