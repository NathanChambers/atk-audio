mod analysis;
mod mixer;
mod plots;
mod synthesis;
use crate::{
    Result,
    project::{Project, validate_id},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, time::Duration};

const RATE: f64 = 48000.;
struct Audio {
    channels: Vec<Vec<f32>>,
    rate: f64,
}
impl Audio {
    fn duration(&self) -> f64 {
        self.channels[0].len() as f64 / self.rate
    }
}
fn number(v: &Value, key: &str, default: f64, min: f64, max: f64) -> Result<f64> {
    let n = if v[key].is_null() {
        default
    } else {
        v[key].as_f64().unwrap_or(f64::NAN)
    };
    if !(min..=max).contains(&n) {
        return Err(format!("{key} must be between {min} and {max}").into());
    }
    Ok(n)
}
fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or_default()
}
fn option_str<'a>(options: &'a Value, key: &str, default: &'a str) -> Result<&'a str> {
    if options[key].is_null() {
        return Ok(default);
    }
    options[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string").into())
}
fn num(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or_default()
}
fn find<'a>(v: &'a Value, k: &str, id: &Value) -> Result<&'a Value> {
    list(v, k)
        .iter()
        .find(|item| item["id"] == *id)
        .ok_or_else(|| format!("Unknown ID: {id}").into())
}

fn read_source(location: &Value) -> Result<Vec<u8>> {
    let location = location.as_str().ok_or("Provide a WAV path or URL")?;
    let lower = location.to_ascii_lowercase();
    let reader: Box<dyn Read> = if lower.starts_with("https://") || lower.starts_with("http://") {
        Box::new(
            ureq::get(location)
                .timeout(Duration::from_secs(30))
                .call()?
                .into_reader(),
        )
    } else {
        Box::new(fs::File::open(location)?)
    };
    let mut bytes = Vec::new();
    let limit = 64 * 1024 * 1024;
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("WAV exceeds 64 MiB".into());
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<Audio> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("Expected RIFF WAV".into());
    }
    let mut offset = 12;
    let mut fmt = None;
    let mut data = None;
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into()?) as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or("Truncated WAV")?;
        let chunk = bytes.get(start..end).ok_or("Truncated WAV")?;
        match &bytes[offset..offset + 4] {
            b"fmt " if size >= 16 => fmt = Some(chunk),
            b"data" => data = Some(chunk),
            _ => (),
        }
        offset = end + size % 2;
    }
    let fmt = fmt.ok_or("Expected PCM or 32-bit float WAV")?;
    let data = data.ok_or("Missing WAV data")?;
    let word = |i| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
    let (encoding, count, bits) = (word(0), word(2) as usize, word(14));
    let rate = u32::from_le_bytes(fmt[4..8].try_into()?) as f64;
    let width = usize::from(bits / 8);
    if !(encoding == 1 && [8, 16, 24, 32].contains(&bits) || encoding == 3 && bits == 32) {
        return Err("Expected PCM or 32-bit float WAV".into());
    }
    if !(1..=2).contains(&count)
        || !(8000.0..=192000.).contains(&rate)
        || usize::from(word(12)) != width * count
    {
        return Err("Expected mono or stereo WAV at 8–192 kHz".into());
    }
    let frames = data.len() / (width * count);
    if frames == 0 || frames as f64 > 120. * rate {
        return Err("WAV must contain up to 120 seconds of audio".into());
    }
    let mut channels = vec![Vec::with_capacity(frames); count];
    for frame in data.chunks_exact(width * count) {
        for (channel, b) in channels.iter_mut().zip(frame.chunks_exact(width)) {
            let sample = match (encoding, bits) {
                (3, _) => f32::from_le_bytes(b.try_into()?),
                (_, 8) => (b[0] as f32 - 128.) / 128.,
                (_, 16) => i16::from_le_bytes(b.try_into()?) as f32 / 32768.,
                (_, 24) => i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32 / 2147483648.,
                _ => i32::from_le_bytes(b.try_into()?) as f32 / 2147483648.,
            };
            if !sample.is_finite() {
                return Err("WAV contains invalid samples".into());
            }
            channel.push(sample.clamp(-1., 1.));
        }
    }
    Ok(Audio { channels, rate })
}
fn encode(audio: &Audio) -> Vec<u8> {
    let count = audio.channels.len() as u16;
    let size = (audio.channels[0].len() * usize::from(count) * 2) as u32;
    let mut out = Vec::with_capacity(44 + size as usize);
    out.extend(b"RIFF");
    out.extend((36 + size).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend((audio.rate as u32).to_le_bytes());
    out.extend((audio.rate as u32 * u32::from(count) * 2).to_le_bytes());
    out.extend((count * 2).to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(size.to_le_bytes());
    for i in 0..audio.channels[0].len() {
        for channel in &audio.channels {
            let sample = f64::from(channel[i]).clamp(-1., 1.);
            let value = sample * if sample < 0. { 32768. } else { 32767. };
            out.extend(((value + 0.5).floor() as i16).to_le_bytes());
        }
    }
    out
}
pub fn source(command: &Value) -> Result<(Value, Vec<u8>)> {
    let id = command["id"].as_str().unwrap_or_default();
    validate_id(id)?;
    let (bytes, provenance) = match command["command"].as_str() {
        Some("source_generate") => {
            let (audio, settings) = synthesis::generate(command)?;
            (encode(&audio), json!({"settings":settings}))
        }
        Some("melody_import") => {
            let input = decode(&read_source(&command["location"])?)?;
            let notes = analysis::melody(&input)?;
            let last = notes.last().ok_or("No clear sung notes found")?;
            let mut samples =
                vec![0.; ((num(last, "start") + num(last, "duration")) * RATE).round() as usize];
            for note in &notes {
                let hz = 440. * 2f64.powf((num(note, "midi") - 69.) / 12.);
                let (audio, _) = synthesis::generate(
                    &json!({"frequency":hz,"seconds":note["duration"],"gain":0.65,"harmonics":3,"attack":0.008,"release":0.015}),
                )?;
                let start = (num(note, "start") * RATE).round() as usize;
                let quantized = decode(&encode(&audio))?;
                for (dest, src) in samples[start..].iter_mut().zip(&quantized.channels[0]) {
                    *dest = *src;
                }
            }
            (
                encode(&Audio {
                    channels: vec![samples],
                    rate: RATE,
                }),
                json!({"location":command["location"],"notes":notes}),
            )
        }
        _ => {
            let mut p = json!({"location":command["location"]});
            for k in ["license", "attribution"] {
                p[k] = command
                    .get(k)
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or(json!(""))
            }
            (read_source(&command["location"])?, p)
        }
    };
    let audio = decode(&bytes)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    Ok((
        json!({"id":id,"hash":hash,"duration":audio.duration(),"rate":audio.rate as u32,"channels":audio.channels.len(),"prompt":command.get("prompt").filter(|v|!v.is_null()).cloned().unwrap_or(json!("")),"provenance":provenance}),
        bytes,
    ))
}
pub use mixer::process;
