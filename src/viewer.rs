use crate::{Result, project::Project};
use serde_json::Value;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub fn parse_port(port: Option<&str>) -> Result<u16> {
    port.unwrap_or("7880")
        .parse()
        .map_err(|_| "Invalid viewer port".into())
}

pub fn serve(project: &Project, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("Audio Workbench viewer: http://{}", listener.local_addr()?);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = handle(stream, project);
    }

    Ok(())
}

fn handle(mut stream: TcpStream, project: &Project) -> Result<()> {
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
