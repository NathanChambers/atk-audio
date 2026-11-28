use crate::{Result, project::Project};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

type DocumentStamps = BTreeMap<(&'static str, OsString), (u64, Option<SystemTime>)>;

pub fn parse_port(port: Option<&str>) -> Result<u16> {
    port.unwrap_or("0")
        .parse()
        .map_err(|_| "Invalid viewer port".into())
}

pub fn serve(project: &Project, port: u16) -> Result<()> {
    serve_with_options(project, port, true)
}

pub fn serve_with_options(project: &Project, port: u16, _headless: bool) -> Result<()> {
    serve_with_lan(project, port, false)
}

pub fn serve_with_lan(project: &Project, port: u16, lan: bool) -> Result<()> {
    let host = if lan {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    let listener = TcpListener::bind((host, port))?;
    let (event_streams, pending) = mpsc::sync_channel(16);
    let watched_project = Project::new(project.root.clone());
    thread::spawn(move || stream_changes(&watched_project, pending));

    let port = listener.local_addr()?.port();
    println!("atk-audio viewer: http://127.0.0.1:{port}");
    if lan {
        eprintln!("LAN access: open http://<host-LAN-IP>:{port} from another device.");
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = handle(stream, project, &event_streams);
    }

    Ok(())
}

fn handle(
    mut stream: TcpStream,
    project: &Project,
    event_streams: &SyncSender<TcpStream>,
) -> Result<()> {
    let timeout = Some(Duration::from_secs(2));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;

    let mut reader = BufReader::new((&mut stream).take(16 * 1024));
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut parts = request.split_ascii_whitespace();
    let method = parts.next();
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or(target);
    let mut tag = None;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("if-none-match")
        {
            tag = Some(value.trim().to_owned());
        }
    }

    if method != Some("GET") {
        return Ok(send(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"Method not allowed",
            None,
        )?);
    }

    if let Some((content_type, bytes)) = static_file(path) {
        return Ok(send(&mut stream, "200 OK", content_type, bytes, None)?);
    }

    if path == "/api/events" {
        let _ = event_streams.try_send(stream);
        return Ok(());
    }

    let segments: Vec<_> = path.split('/').collect();
    let snapshot = match segments.as_slice() {
        ["", "api", "project"] => project.list(),
        ["", "api", kind @ ("sessions" | "assets"), id] => project.document(kind, id),
        ["", "api", "sources", hash] => {
            if let Ok(file) = project.source_path(hash)
                && let Ok(bytes) = fs::read(file)
            {
                return Ok(send(&mut stream, "200 OK", "audio/wav", &bytes, None)?);
            }
            return Ok(send(
                &mut stream,
                "404 Not Found",
                "text/plain",
                b"Source not found",
                None,
            )?);
        }
        _ => {
            return Ok(send(
                &mut stream,
                "404 Not Found",
                "text/plain",
                b"Page not found",
                None,
            )?);
        }
    };
    match snapshot {
        Ok(value) => send_snapshot(&mut stream, &value, tag.as_deref())?,
        Err(error) => send(
            &mut stream,
            "404 Not Found",
            "text/plain",
            error.to_string().as_bytes(),
            None,
        )?,
    }

    Ok(())
}

fn stream_changes(project: &Project, pending: Receiver<TcpStream>) {
    let mut clients = Vec::new();
    let mut documents = None;
    let interval = Duration::from_millis(250);
    let mut next_check = Instant::now();
    let mut heartbeat = Instant::now();

    loop {
        let incoming = if clients.is_empty() {
            documents = None;
            pending.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            pending.recv_timeout(next_check.saturating_duration_since(Instant::now()))
        };

        match incoming {
            Ok(mut stream) if clients.len() < 32 => {
                if clients.is_empty() {
                    documents = document_stamps(project).ok();
                    heartbeat = Instant::now() + Duration::from_secs(15);
                }

                let headers = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n: connected\n\n";
                if stream.set_nonblocking(true).is_ok() && stream.write_all(headers).is_ok() {
                    clients.push(stream);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            _ => {}
        }

        let now = Instant::now();
        if clients.is_empty() || now < next_check {
            continue;
        }

        next_check = now + interval;
        if let Ok(current) = document_stamps(project) {
            if documents.as_ref() != Some(&current) {
                documents = Some(current);
                clients
                    .retain_mut(|stream| stream.write_all(b"event: change\ndata: {}\n\n").is_ok());
            }
        }

        if now >= heartbeat {
            heartbeat = now + Duration::from_secs(15);
            clients.retain_mut(|stream| stream.write_all(b": keepalive\n\n").is_ok());
        }
    }
}

fn document_stamps(project: &Project) -> io::Result<DocumentStamps> {
    let mut documents = BTreeMap::new();
    for kind in ["assets", "sessions"] {
        let path = project.root.join(".audio").join(kind);
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }

            let metadata = entry.metadata()?;
            documents.insert(
                (kind, entry.file_name()),
                (metadata.len(), metadata.modified().ok()),
            );
        }
    }

    Ok(documents)
}

fn static_file(path: &str) -> Option<(&'static str, &'static [u8])> {
    let javascript = "text/javascript; charset=utf-8";
    Some(match path {
        "/" => (
            "text/html; charset=utf-8",
            include_bytes!("../web/viewer.html"),
        ),
        "/viewer.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../web/viewer.css"),
        ),
        "/viewer.mjs" => (javascript, include_bytes!("../web/viewer.mjs")),
        "/playback.mjs" => (javascript, include_bytes!("../web/playback.mjs")),
        "/audio.mjs" => (javascript, include_bytes!("../web/audio.mjs")),
        "/tune.mjs" => (javascript, include_bytes!("../web/tune.mjs")),
        _ => return None,
    })
}

fn send_snapshot(stream: &mut TcpStream, value: &Value, previous_tag: Option<&str>) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let tag = format!("\"{:016x}\"", hasher.finish());
    let content_type = "application/json; charset=utf-8";
    if previous_tag == Some(&tag) {
        send(stream, "304 Not Modified", content_type, b"", Some(&tag))?;
    } else {
        send(stream, "200 OK", content_type, &bytes, Some(&tag))?;
    }

    Ok(())
}

fn send(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    tag: Option<&str>,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n",
        body.len()
    )?;
    if let Some(tag) = tag {
        write!(stream, "ETag: {tag}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body)
}
