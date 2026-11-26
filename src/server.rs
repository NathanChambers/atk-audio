use atk_audio::{Result, VERSION, project::Project, viewer};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("atk-audio: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut headless = false;
    let mut project = None;
    let mut port = None;
    let mut arguments = env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--project") => {
                let directory = arguments.next().ok_or("Missing project directory")?;
                set_project(&mut project, directory.into())?;
            }
            Some("--headless") => headless = true,
            Some("--port") => {
                port = Some(
                    arguments
                        .next()
                        .ok_or("Missing viewer port")?
                        .into_string()
                        .map_err(|_| "Invalid viewer port")?,
                )
            }
            Some("--version" | "-V") => {
                println!("atk-audio {VERSION}");
                return Ok(());
            }
            Some("--help" | "-h") => {
                println!(
                    "atk-audio [--project DIR | DIR] [--port PORT] [--headless]\nUse --headless to print the viewer URL without launching a browser.\nServes saved audio sessions and assets in a read-only browser viewer.\nThe directory defaults to the current working directory. Omit --port or pass 0 for an available port."
                );
                return Ok(());
            }
            _ if !argument.as_encoded_bytes().starts_with(b"-") => {
                set_project(&mut project, argument.into())?;
            }
            _ => return Err(format!("Unknown option: {}", argument.to_string_lossy()).into()),
        }
    }

    let project = Project::new(project.unwrap_or_else(|| PathBuf::from(".")));
    viewer::serve_with_options(&project, viewer::parse_port(port.as_deref())?, headless)
}

fn set_project(project: &mut Option<PathBuf>, directory: PathBuf) -> Result<()> {
    if project.is_some() {
        return Err("Specify the project directory once, as a path or with --project".into());
    }

    *project = Some(directory);
    Ok(())
}
