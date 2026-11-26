use atk_audio::project::unique_name;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};

struct TestProject(PathBuf);

impl TestProject {
    fn create() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(unique_name());
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn command(&self, session: &str, input: Value, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_atk-audio-cli"))
            .env("PATH", "")
            .current_dir(&self.0)
            .args(["--session", session, "--command", &input.to_string()])
            .args(extra)
            .output()
            .unwrap()
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Viewer(Child);

impl Drop for Viewer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn replies(output: Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    BufReader::new(output.stdout.as_slice())
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect()
}

#[test]
fn instrument_sources_preserve_the_reference_wavs() {
    let project = TestProject::create();
    let references = [
        ("muted-pluck", "9262f0dd166047d900bf3da803b114f191c66c397f6ec03faabf83c1f771d93a"),
        ("rounded-bass", "f20ffe2de095842fdef825b2a4c307b9d3bb5847b48eb06e5eab23d37689ccde"),
        ("ceramic-tap", "af0e44ecd2cc27ef986f00c52220fdb212e91ecedd01a577efe822dbd709e95b"),
    ];
    let mut commands = vec![json!({"command":"session_start","asset":"instruments"})];
    for (instrument, _) in references {
        commands.push(json!({"command":"source_generate","instrument":instrument,"id":instrument}));
    }

    let results = replies(project.command("reference", json!(commands), &[]));
    for (result, (instrument, hash)) in results[1..].iter().zip(references) {
        assert_eq!(result["result"]["hash"], hash);
        assert_eq!(result["result"]["provenance"]["settings"]["instrument"], instrument);
    }
}

#[test]
fn native_cli_edits_renders_and_checks_publish_revisions() {
    let project = TestProject::create();
    let start = json!({"command": "session_start", "asset": "pickup"});
    let steps = json!([
        start,
        {"command": "source_generate", "id": "tone", "seconds": 0.5},
        {"command": "track_add", "id": "lead"},
        {"command": "track_add", "id": "other"},
        {"command": "clip_add", "id": "hit", "track": "lead", "source": "tone"},
        {"command": "control_add", "id": "level", "type": "slider"},
        {"command": "mapping_set", "clip": "hit", "control": "level", "target": "gain", "min": 0.2, "max": 1},
        {"command": "clip_split", "id": "hit", "newId": "tail", "time": 0.25},
        {"command": "clip_edit", "id": "tail", "track": "other"},
        {"command": "preset_set", "id": "test", "seconds": 0.5, "lanes": [{"control": "level", "points": [{"time": 0, "value": 1}]}]}
    ]);
    assert_eq!(replies(project.command("first", steps, &[])).len(), 10);
    let result = replies(project.command(
        "first",
        json!({"command": "session_render", "preset": "test"}),
        &["--output", "pickup.wav"],
    ));
    assert_eq!(result[0]["output"], "pickup.wav");
    assert_eq!(
        result[0]["written"]["bytes"],
        fs::metadata(project.0.join("pickup.wav")).unwrap().len()
    );
    assert!(
        fs::read(project.0.join("pickup.wav"))
            .unwrap()
            .starts_with(b"RIFF")
    );

    replies(project.command("second", start, &[]));
    let publish = json!({"command": "session_publish"});
    let result = replies(project.command("first", publish.clone(), &[]));
    assert_eq!(result[0]["result"]["revision"], 1);
    let conflict = project.command("second", publish, &[]);
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("Publish conflict"));

    let draft = replies(project.command("first", json!({"command": "session_inspect"}), &[]));
    let asset = &draft[0]["result"]["asset"];
    assert_eq!(asset["tracks"][1]["clips"][0]["id"], "tail");
    assert_eq!(asset["mappings"][1]["clip"], "tail");
}

#[test]
fn melody_import_preserves_detected_notes_and_rests() {
    let project = TestProject::create();
    let steps = json!([
        {"command": "session_start", "asset": "reference"},
        {"command": "source_generate", "id": "low", "frequency": 220, "seconds": 0.25},
        {"command": "source_generate", "id": "high", "frequency": 440, "seconds": 0.25},
        {"command": "track_add", "id": "reference"},
        {"command": "clip_add", "id": "low", "track": "reference", "source": "low", "start": 0.2},
        {"command": "clip_add", "id": "high", "track": "reference", "source": "high", "start": 0.6},
        {"command": "session_render"}
    ]);
    replies(project.command("reference", steps, &["--output", "reference.wav"]));
    let import =
        json!({"command": "melody_import", "id": "sung", "location": "reference.wav", "start": 1});
    let steps = json!([
        {"command": "session_start", "asset": "song"},
        import,
        {"command": "session_inspect"}
    ]);
    let result = replies(project.command("draft", steps, &[]));
    let notes = &result[1]["result"]["provenance"]["notes"];
    assert_eq!(notes.as_array().unwrap().len(), 2);
    assert_eq!(notes[0]["midi"], 57);
    assert_eq!(notes[1]["midi"], 69);
    let rest = notes[1]["start"].as_f64().unwrap() - notes[0]["duration"].as_f64().unwrap();
    assert!(rest > 0.1 && rest < 0.2);

    let asset = &result[2]["result"]["asset"];
    assert_eq!(asset["tracks"][0]["clips"][0]["start"], 1.0);
    assert!(!project.command("draft", import, &[]).status.success());
    let draft = replies(project.command("draft", json!({"command": "session_inspect"}), &[]));
    assert_eq!(draft[0], result[2]);
    replies(project.command(
        "draft",
        json!({"command": "session_render"}),
        &["--output", "melody.wav"],
    ));
}

#[test]
fn viewer_reads_snapshots_and_rejects_writes() {
    let project = TestProject::create();
    let start = json!({"command": "session_start", "asset": "pickup"});
    replies(project.command("draft", start, &[]));
    let mut viewer = Viewer(
        Command::new(env!("CARGO_BIN_EXE_atk-audio"))
            .arg("--headless")
            .current_dir(project.0.parent().unwrap())
            .args(["--port", "0"])
            .arg(project.0.file_name().unwrap())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut address = String::new();
    BufReader::new(viewer.0.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let address = address.trim().split("http://").last().unwrap();

    let mut events = open_event_stream(address);
    let page = request(address, "GET / HTTP/1.1\r\n\r\n");
    assert!(page.contains("/viewer.mjs"));
    assert!(page.contains(r#"aria-label="Record audio""#));
    assert!(request(address, "GET /tune.mjs HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 200"));
    let snapshot = request(address, "GET /api/sessions/draft HTTP/1.1\r\n\r\n");
    assert!(snapshot.starts_with("HTTP/1.1 200"));
    assert!(snapshot.contains("\"baseRevision\":0"));
    let tag = snapshot
        .lines()
        .find_map(|line| line.strip_prefix("ETag: "))
        .unwrap();
    let cached = request(
        address,
        &format!("GET /api/sessions/draft HTTP/1.1\r\nIf-None-Match: {tag}\r\n\r\n"),
    );
    assert!(cached.starts_with("HTTP/1.1 304"));

    let rejected = request(
        address,
        "POST /api/sessions/draft HTTP/1.1\r\nContent-Length: 0\r\n\r\n",
    );
    assert!(rejected.starts_with("HTTP/1.1 405"));
    assert_eq!(
        snapshot,
        request(address, "GET /api/sessions/draft HTTP/1.1\r\n\r\n")
    );

    replies(project.command("draft", json!({"command": "session_publish"}), &[]));
    let mut event = String::new();
    events.read_line(&mut event).unwrap();
    assert_eq!(event, "event: change\n");
    let published = request(address, "GET /api/assets/pickup HTTP/1.1\r\n\r\n");
    assert!(published.starts_with("HTTP/1.1 200"));
    assert!(published.contains("\"revision\":1"));

    drop(events);
    replies(project.command(
        "second",
        json!({"command": "session_start", "asset": "new"}),
        &[],
    ));
    let _reconnected = open_event_stream(address);
    let project = request(address, "GET /api/project HTTP/1.1\r\n\r\n");
    assert!(project.contains("\"id\":\"second\""));
}

fn open_event_stream(address: &str) -> BufReader<TcpStream> {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(b"GET /api/events HTTP/1.1\r\n\r\n")
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut headers = String::new();
    for line in reader.by_ref().lines() {
        let line = line.unwrap();
        if line.is_empty() {
            break;
        }
        headers.push_str(&line);
    }
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert!(headers.contains("Content-Type: text/event-stream"));
    let mut ready = String::new();
    reader.read_line(&mut ready).unwrap();
    assert_eq!(ready, ": connected\n");
    ready.clear();
    reader.read_line(&mut ready).unwrap();
    assert_eq!(ready, "\n");
    reader
}

fn request(address: &str, input: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    stream.write_all(input.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn draft_edits_advance_the_draft_revision() {
    let project = TestProject::create();
    let start = json!({"command": "session_start", "asset": "pickup"});
    replies(project.command("draft", start, &[]));
    let track = json!({"command": "track_add", "id": "main"});
    replies(project.command("draft", track.clone(), &[]));
    assert!(!project.command("draft", track, &[]).status.success());

    let draft = replies(project.command("draft", json!({"command": "session_inspect"}), &[]));
    assert_eq!(draft[0]["result"]["draftRevision"], 1);
    assert_eq!(draft[0]["result"]["baseRevision"], 0);
}
