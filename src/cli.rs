mod engine;
mod recording;
#[cfg(windows)]
mod loopback;

use audio_workshop::{Result, VERSION, edits, project::Project, viewer};
use engine::Engine;
use serde_json::{Value, json};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const HELP: &str = "atk-audio-cli [--project DIR] [--session NAME] [--batch] [--file FILE | --input FILE | --command JSON] [--output FILE]\n\
atk-audio-cli --project DIR --session audio-validation-suite --validation-suite\n\
atk-audio [--project DIR | DIR] [--port 7880]\n\
Commands can also be piped as JSON. Submit a command array to run commands in order.\n\
Use --batch to require an array. Send {\"command\":\"help\"} for command discovery.\n\
Use --debug to record requests, replies, and saved session state. Use --replay FILE to read a recording.\n\
On Windows, capture_system records the playback device to a WAV with --output FILE.\n\
Use spectrogram for a PNG heatmap or audio_analyze with format waveform, features, mel, pcm or impulse.";

#[derive(Default)]
struct Options {
    project: PathBuf,
    session: String,
    file: Option<PathBuf>,
    command: Option<String>,
    output: Option<PathBuf>,
    port: Option<String>,
    validation: bool,
    serve: bool,
    help: bool,
    version: bool,
    batch: bool,
    debug: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let reply = json!({"ok": false, "error": error.to_string()});
            println!("{reply}");
            eprintln!("{reply}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut arguments = env::args_os().skip(1).peekable();
    if arguments.peek().is_some_and(|argument| argument == "--replay") {
        arguments.next();
        let path = arguments.next().ok_or("--replay requires a recording file")?;
        let frame = match arguments.next() {
            None => None,
            Some(option) if option == "--frame" => {
                let value = arguments.next().ok_or("--frame requires a nonnegative number")?;
                Some(value.to_str().and_then(|value| value.parse().ok()).ok_or("--frame requires a nonnegative number")?)
            }
            _ => return Err("Use --replay FILE [--frame NUMBER]".into()),
        };
        if arguments.next().is_some() {
            return Err("Use --replay FILE [--frame NUMBER]".into());
        }
        return recording::replay(Path::new(&path), frame);
    }

    let options = parse_options()?;
    if options.version {
        println!("atk-audio-cli {VERSION}");
        return Ok(());
    }

    if options.help {
        println!("{HELP}");
        return Ok(());
    }

    let project = Project::new(options.project.clone());
    if options.serve {
        if options.debug {
            return Err("Use --debug with commands, not with --serve".into());
        }
        return viewer::serve(&project, viewer::parse_port(options.port.as_deref())?);
    }

    let mut engine = None;
    let input = if options.validation {
        decode_commands(include_bytes!("fixtures/validation-suite.json"))?
    } else if let Some(path) = &options.file {
        decode_commands(&fs::read(path)?)?
    } else if let Some(command) = &options.command {
        decode_commands(command.as_bytes())?
    } else {
        if io::stdin().is_terminal() {
            return Err(
                "Provide JSON commands, use --help, or run atk-audio for the viewer".into(),
            );
        }

        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        decode_commands(&bytes)?
    };

    let commands = match input {
        Value::Array(commands) if !options.batch || (1..=1024).contains(&commands.len()) => commands,
        _ if options.batch => return Err("Batch input must be a JSON array of 1..1024 commands".into()),
        command => vec![command],
    };
    let mut recording = if options.debug {
        let directory = project.root.join(".audio").join("recordings");
        let state = recorded_state(&project, &options.session, &Value::Null);
        Some(recording::Recording::create(&directory, &options.session, state)?)
    } else {
        None
    };
    for command in commands {
        if let Some(recording) = &mut recording {
            recording.begin(&command)?;
        }
        let result = execute_with_receipt(&project, &options, &mut engine, &command);
        if let Some(recording) = &mut recording {
            let state = recorded_state(&project, &options.session, &command);
            let reply = match &result {
                Ok(reply) => reply.clone(),
                Err(error) => json!({"ok": false, "error": error.to_string()}),
            };
            recording.finish(&reply, state)?;
        }
        let response = result?;
        println!("{response}");
    }

    Ok(())
}

fn recorded_state(project: &Project, session: &str, command: &Value) -> Value {
    let draft = project.document("sessions", session).ok();
    let asset_id = command["asset"]
        .as_str()
        .or_else(|| draft.as_ref().and_then(|draft| draft["asset"]["id"].as_str()));
    let asset = asset_id.and_then(|id| project.document("assets", id).ok());
    json!({"draft": draft, "asset": asset})
}

fn parse_options() -> Result<Options> {
    let mut options = Options {
        project: PathBuf::from("."),
        ..Options::default()
    };
    let mut arguments = env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let argument = argument.to_str().ok_or("Expected a UTF-8 option name")?;
        let (name, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(name, value)| (name, Some(value)));
        let mut next_value = || {
            inline
                .map(Into::into)
                .or_else(|| arguments.next())
                .ok_or_else(|| format!("Missing value for {name}"))
        };
        match name {
            "--project" => options.project = PathBuf::from(next_value()?),
            "--session" => {
                options.session = next_value()?
                    .into_string()
                    .map_err(|_| "Invalid session ID")?
            }
            "--file" | "--input" | "-i" => options.file = Some(PathBuf::from(next_value()?)),
            "--command" => {
                options.command = Some(
                    next_value()?
                        .into_string()
                        .map_err(|_| "Expected a UTF-8 JSON command")?,
                )
            }
            "--output" => options.output = Some(PathBuf::from(next_value()?)),
            "--port" => {
                options.port = Some(
                    next_value()?
                        .into_string()
                        .map_err(|_| "Invalid viewer port")?,
                )
            }
            "--validation-suite" if inline.is_none() => options.validation = true,
            "--serve" if inline.is_none() => options.serve = true,
            "--batch" if inline.is_none() => options.batch = true,
            "--debug" if inline.is_none() => options.debug = true,
            "--help" | "-h" if inline.is_none() => options.help = true,
            "--version" | "-V" if inline.is_none() => options.version = true,
            _ => return Err(format!("Unknown option: {argument}").into()),
        }
    }

    Ok(options)
}

fn decode_commands(bytes: &[u8]) -> Result<Value> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    Ok(serde_json::from_slice(bytes)?)
}

fn execute_with_receipt(
    project: &Project,
    options: &Options,
    engine: &mut Option<Engine>,
    command: &Value,
) -> Result<Value> {
    let mut reply = execute(project, options, engine, command)?;
    if let Some(path) = reply["output"].as_str() {
        let receipt = json!({"path": fs::canonicalize(path)?, "bytes": fs::metadata(path)?.len()});
        let mut result = reply.as_object().expect("command response object").clone();
        result.remove("ok");
        reply["result"] = Value::Object(result);
        reply["written"] = receipt;
    }
    Ok(reply)
}

fn execute(
    project: &Project,
    options: &Options,
    engine: &mut Option<Engine>,
    command: &Value,
) -> Result<Value> {
    if !command.is_object() {
        return Err("Expected a JSON command".into());
    }

    if command["command"] == "capture_system" {
        return capture_system(options, command);
    }

    let session = &options.session;
    let result = match command["command"].as_str() {
        Some("help") => command_help(),
        Some("project_list") => project.list()?,
        Some("session_inspect") => project.document("sessions", session)?,
        Some("asset_inspect") => {
            project.document("assets", command["asset"].as_str().unwrap_or_default())?
        }
        Some("session_start" | "session_publish") => {
            project.update(session, command, |_| unreachable!())?
        }
        Some("session_render" | "spectrogram" | "audio_analyze") => {
            return process(project, options, engine, command);
        }
        Some("melody_import") => {
            let (source, bytes) =
                worker(engine)?.run(&json!({"operation": "source", "command": command}))?;
            project.update(session, command, |asset| {
                edits::add_melody(asset, command, &source)?;
                project.store_source(&source, &bytes)?;
                Ok(source)
            })?
        }
        Some("source_generate" | "source_import") => {
            let (source, bytes) =
                worker(engine)?.run(&json!({"operation": "source", "command": command}))?;
            project.update(session, command, |asset| {
                let result = edits::add(&mut asset["sources"], source)?;
                project.store_source(&result, &bytes)?;
                Ok(result)
            })?
        }
        _ => project.update(session, command, |asset| edits::edit(asset, command))?,
    };

    Ok(json!({"ok": true, "result": result}))
}

fn command_help() -> Value {
    json!({
        "commands": {
            "help": {},
            "project_list": {},
            "session_start": {"asset": "asset ID", "name": "display name"},
            "session_inspect": {},
            "session_publish": {},
            "asset_inspect": {"asset": "asset ID"},
            "asset_name": {"name": "display name"},
            "source_generate": {},
            "source_import": {},
            "melody_import": {"id": "source ID", "location": "WAV path"},
            "track_add": {"id": "track ID"},
            "track_edit": {"id": "track ID"},
            "clip_add": {"id": "clip ID", "track": "track ID", "source": "source ID"},
            "clip_edit": {"id": "clip ID"},
            "clip_remove": {"id": "clip ID"},
            "clip_split": {"id": "clip ID"},
            "control_add": {"id": "control ID", "type": "slider or trigger"},
            "mapping_set": {},
            "preset_set": {},
            "preset_remove": {"id": "preset ID"},
            "session_render": {},
            "spectrogram": {},
            "audio_analyze": {},
            "capture_system": {"seconds": 15}
        },
        "examples": [{"command": "project_list"}, {"command": "session_inspect"}],
        "notes": [
            "Pass --session ID to select a draft.",
            "Command arrays stop at the first failure and keep earlier edits.",
            "Publish checks the saved draft against the current asset revision.",
            "See the Audio Workbench README for command examples and processing options."
        ]
    })
}

#[cfg(windows)]
fn capture_system(options: &Options, command: &Value) -> Result<Value> {
    let output = options
        .output
        .as_deref()
        .ok_or("Use --output for a WAV recording")?;
    if output.try_exists()? {
        return Err(format!("Output already exists: {}", output.display()).into());
    }

    let seconds = command.get("seconds").map_or(Some(15.0), Value::as_f64);
    let seconds = seconds
        .filter(|seconds| (0.25..=30.0).contains(seconds))
        .ok_or("Recording seconds must be between 0.25 and 30")?;
    let device = match command.get("device") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|id| !id.is_empty() && !id.contains('\0'))
                .ok_or("Expected a Windows playback endpoint ID")?,
        ),
    };
    let recording = loopback::record(seconds, device)?;
    write_output(output, &recording.bytes)?;
    Ok(
        json!({"ok": true, "output": output, "duration": recording.duration, "rate": recording.rate, "channels": recording.channels}),
    )
}

#[cfg(not(windows))]
fn capture_system(_: &Options, _: &Value) -> Result<Value> {
    Err("Native system audio capture is available on Windows".into())
}

fn process(
    project: &Project,
    options: &Options,
    engine: &mut Option<Engine>,
    command: &Value,
) -> Result<Value> {
    let required = match (command["command"].as_str(), command["format"].as_str()) {
        (Some("session_render"), _) => Some("Use --output for a WAV render"),
        (Some("spectrogram"), _) => Some("Use --output for a PNG spectrogram"),
        (Some("audio_analyze"), Some("waveform")) => Some("Use --output for a waveform PNG"),
        _ => None,
    };
    if let (None, Some(message)) = (&options.output, required) {
        return Err(message.into());
    }

    let request = json!({"operation": "process", "command": command, "project": project.root, "session": options.session});
    let (mut result, bytes) = worker(engine)?.run(&request)?;
    let Some(output) = &options.output else {
        return Ok(json!({"ok": true, "result": result}));
    };

    if !bytes.is_empty() {
        write_output(output, &bytes)?;
        result["ok"] = json!(true);
        result["output"] = json!(output);
        return Ok(result);
    }

    let data = format!("{result}\n");
    write_output(output, data.as_bytes())?;
    Ok(json!({"ok": true, "format": result["format"], "scope": result["scope"], "output": output}))
}

fn write_output(path: &Path, bytes: &[u8]) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

fn worker(engine: &mut Option<Engine>) -> Result<&Engine> {
    if engine.is_none() {
        *engine = Some(Engine::create()?);
    }

    Ok(engine.as_ref().expect("initialized engine"))
}
