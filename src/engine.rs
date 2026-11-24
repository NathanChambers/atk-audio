use audio_workshop::{Result, project::unique_name};
use serde_json::Value;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

const FILES: &[(&str, &[u8])] = &[
    ("src/worker/worker.mjs", include_bytes!("worker/worker.mjs")),
    (
        "src/worker/project.mjs",
        include_bytes!("worker/project.mjs"),
    ),
    ("src/worker/audio.mjs", include_bytes!("worker/audio.mjs")),
    (
        "src/worker/analysis.mjs",
        include_bytes!("worker/analysis.mjs"),
    ),
    (
        "src/worker/spectrogram.mjs",
        include_bytes!("worker/spectrogram.mjs"),
    ),
    ("web/audio.mjs", include_bytes!("../web/audio.mjs")),
];

pub struct Engine(PathBuf);

impl Engine {
    pub fn create() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("audio-workshop-{}", unique_name()));
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder.create(&path)?;

        let engine = Self(path);
        fs::create_dir_all(engine.0.join("src/worker"))?;
        fs::create_dir(engine.0.join("web"))?;
        for (name, bytes) in FILES {
            fs::write(engine.0.join(name), bytes)?;
        }

        Ok(engine)
    }

    pub fn run(&self, request: &Value) -> Result<(Value, Vec<u8>)> {
        let input = serde_json::to_vec(request)?;
        let mut child = Command::new("node")
            .arg(self.0.join("src/worker/worker.mjs"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| match error.kind() {
                io::ErrorKind::NotFound => io::Error::new(
                    error.kind(),
                    "Audio processing requires Node.js 20 or newer on PATH",
                ),
                _ => error,
            })?;
        let written = child.stdin.take().expect("piped stdin").write_all(&input);
        let mut output = child.wait_with_output()?;
        written?;
        if !output.status.success() {
            return Err(format!("Audio engine failed with {}", output.status).into());
        }

        let newline = output
            .stdout
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or("Missing audio engine response")?;
        let mut header: Value = serde_json::from_slice(&output.stdout[..newline])?;
        if let Some(error) = header["error"].as_str() {
            return Err(error.to_owned().into());
        }

        let bytes = output.stdout.split_off(newline + 1);
        Ok((header["result"].take(), bytes))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
