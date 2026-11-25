use super::*;
use std::collections::HashMap;

struct Event<'a> {
    frame: usize,
    id: &'a str,
    value: f64,
}
struct Mapping {
    control: usize,
    low: f64,
    high: f64,
    min: f64,
    max: f64,
    power: bool,
    gain: bool,
}
struct Voice<'a> {
    source: &'a Audio,
    trigger: Option<&'a str>,
    start: usize,
    position: f64,
    trim: f64,
    span: f64,
    duration: f64,
    looped: bool,
    fade_in: f64,
    fade_out: f64,
    gains: [f64; 2],
    mappings: Vec<Mapping>,
}
struct Filter {
    b0: f64,
    b1: f64,
    a1: f64,
    a2: f64,
    first: [f64; 2],
    second: [f64; 2],
}
impl Filter {
    fn new(hz: f64, high: bool) -> Self {
        let angle = std::f64::consts::TAU * hz.min(RATE * 0.45) / RATE;
        let alpha = angle.sin() / 2f64.sqrt();
        let cosine = angle.cos();
        let div = 1. + alpha;
        let b0 = (if high { 1. + cosine } else { 1. - cosine }) / (2. * div);
        Self {
            b0,
            b1: if high { -2. * b0 } else { 2. * b0 },
            a1: -2. * cosine / div,
            a2: (1. - alpha) / div,
            first: [0.; 2],
            second: [0.; 2],
        }
    }
    fn apply(&mut self, x: f64, ch: usize) -> f64 {
        let y = self.b0 * x + self.first[ch];
        self.first[ch] = self.b1 * x - self.a1 * y + self.second[ch];
        self.second[ch] = self.b0 * x - self.a2 * y;
        y
    }
}
fn mix(
    asset: &Value,
    sources: &HashMap<String, Audio>,
    duration: f64,
    events: &[Event],
    sustain: bool,
) -> Result<Audio> {
    if !(duration > 0. && duration <= 120.) {
        return Err("Invalid render duration".into());
    }
    let frames = (duration * RATE).round().max(1.) as usize;
    let mut output = vec![[0f64; 2]; frames];
    let tracks = list(asset, "tracks");
    let solo = tracks
        .iter()
        .any(|t| t["solo"] == true && t["mute"] != true);
    let controls = list(asset, "controls");
    for track in tracks {
        if track["mute"] == true || (solo && track["solo"] != true) {
            continue;
        }
        let mut filters = Vec::new();
        for (key, high) in [("highPassHz", true), ("lowPassHz", false)] {
            if let Some(hz) = track[key].as_f64() {
                filters.push(Filter::new(hz, high))
            }
        }
        let mut values: Vec<_> = controls.iter().map(|c| num(c, "value")).collect();
        let mut voices = Vec::new();
        for clip in list(track, "clips") {
            let source = sources
                .get(clip["source"].as_str().unwrap_or_default())
                .ok_or("Missing audio source")?;
            let trim = num(clip, "trim") * source.rate;
            let pan = (num(clip, "pan") + num(track, "pan")).clamp(-1., 1.);
            let gain = num(track, "gain") * num(clip, "gain");
            let mut mappings = Vec::new();
            for m in list(asset, "mappings")
                .iter()
                .filter(|m| m["clip"] == clip["id"])
            {
                let control = controls
                    .iter()
                    .position(|c| c["id"] == m["control"])
                    .ok_or("Unknown mapping control")?;
                mappings.push(Mapping {
                    control,
                    low: num(m, "inputMin"),
                    high: num(m, "inputMax"),
                    min: num(m, "min"),
                    max: num(m, "max"),
                    power: m["curve"] == "equal-power",
                    gain: m["target"] == "gain",
                });
            }
            let trigger = clip["trigger"].as_str();
            let looped = clip["loop"] == true;
            voices.push(Voice {
                source,
                trigger,
                start: if trigger.is_some() {
                    usize::MAX
                } else {
                    (num(clip, "start") * RATE).round() as usize
                },
                position: 0.,
                trim,
                span: (source.channels[0].len() as f64 - trim)
                    .min(num(clip, "length") * source.rate),
                duration: if sustain && looped && trigger.is_none() {
                    f64::INFINITY
                } else {
                    num(clip, "duration")
                },
                looped,
                fade_in: num(clip, "fadeIn"),
                fade_out: num(clip, "fadeOut"),
                gains: [gain * (1. - pan).min(1.), gain * (1. + pan).min(1.)],
                mappings,
            });
        }
        let mut events = events.iter().peekable();
        for (frame, dest) in output.iter_mut().enumerate() {
            while events.peek().is_some_and(|e| e.frame <= frame) {
                let event = events.next().unwrap();
                if let Some(index) = controls.iter().position(|c| c["id"] == event.id) {
                    values[index] = event.value
                }
                for voice in &mut voices {
                    if voice.trigger == Some(event.id) {
                        voice.start = frame;
                        voice.position = 0.;
                    }
                }
            }
            let mut bus = [0.; 2];
            for v in &mut voices {
                if frame < v.start || v.span <= 0. || (!v.looped && v.position >= v.span) {
                    continue;
                }
                let elapsed = (frame - v.start) as f64 / RATE;
                if elapsed >= v.duration {
                    continue;
                }
                let mut gain = 1.;
                let mut pitch = 1.;
                for m in &v.mappings {
                    let f = ((values[m.control] - m.low) / (m.high - m.low)).clamp(0., 1.);
                    let value = if m.power {
                        (m.min * m.min * (1. - f) + m.max * m.max * f).sqrt()
                    } else {
                        m.min + (m.max - m.min) * f
                    };
                    if m.gain {
                        gain *= value
                    } else {
                        pitch *= value
                    }
                }
                let pos = v.trim + v.position % v.span;
                let index = pos.floor() as usize;
                let next = if ((index + 1) as f64) < v.trim + v.span {
                    index + 1
                } else if v.looped {
                    v.trim as usize
                } else {
                    index
                };
                let mut fade = 1f64;
                if v.fade_in > 0. {
                    fade = fade.min(elapsed / v.fade_in)
                }
                if v.fade_out > 0. {
                    fade = fade.min((v.duration - elapsed) / v.fade_out)
                }
                for (ch, total) in bus.iter_mut().enumerate() {
                    let samples = &v.source.channels[ch.min(v.source.channels.len() - 1)];
                    if let Some(&first) = samples.get(index) {
                        let second = *samples.get(next).unwrap_or(&first);
                        *total += (first as f64 + (second as f64 - first as f64) * pos.fract())
                            * (gain * fade)
                            * v.gains[ch];
                    }
                }
                v.position += pitch * v.source.rate / RATE;
            }
            for ch in 0..2 {
                for filter in &mut filters {
                    bus[ch] = filter.apply(bus[ch], ch)
                }
                dest[ch] += bus[ch];
            }
        }
    }
    Ok(Audio {
        channels: (0..2)
            .map(|ch| output.iter().map(|s| s[ch] as f32).collect())
            .collect(),
        rate: RATE,
    })
}
pub fn process(project: &Project, session: &str, c: &Value) -> Result<(Value, Vec<u8>)> {
    let render = c["command"] == "session_render";
    let scope = if render {
        "mix"
    } else {
        option_str(c, "scope", "mix")?
    };
    if scope == "wav" {
        if c["command"] == "audio_analyze" && c.get("preset").is_some() {
            return Err("Presets apply to rendered audio".into());
        }
        let audio = decode(&read_source(&c["location"])?)?;
        return analysis::analyze(&audio, c, json!({"scope":scope}));
    }
    let doc = if !render && c.get("asset").is_some() {
        project.document("assets", c["asset"].as_str().unwrap_or_default())?
    } else {
        project.document("sessions", session)?
    };
    let mut asset = doc["asset"].clone();
    let preset = if let Some(id) = c.get("preset") {
        Some(find(&asset, "presets", id)?.clone())
    } else {
        None
    };
    let load = |source: &Value| -> Result<Audio> {
        let path = project.source_path(source["hash"].as_str().ok_or("Invalid source hash")?)?;
        decode(&fs::read(path)?)
    };
    let mut metadata = json!({"scope":scope,"id":c.get("id").unwrap_or(&asset["id"])});
    if scope == "source" {
        if preset.is_some() {
            return Err("Presets apply to rendered audio".into());
        }
        return analysis::analyze(&load(find(&asset, "sources", &c["id"])?)?, c, metadata);
    }
    let mut trigger = Value::Null;
    match scope {
        "clip" | "track" => {
            let mut track = if scope == "track" {
                find(&asset, "tracks", &c["id"])?.clone()
            } else {
                let mut track = list(&asset, "tracks")
                    .iter()
                    .find(|t| list(t, "clips").iter().any(|v| v["id"] == c["id"]))
                    .ok_or("Unknown clip")?
                    .clone();
                let mut clip = find(&track, "clips", &c["id"])?.clone();
                trigger = clip["trigger"].clone();
                clip["start"] = json!(0);
                track["clips"] = json!([clip]);
                track
            };
            track["mute"] = json!(false);
            track["solo"] = json!(false);
            asset["tracks"] = json!([track]);
        }
        "mix" => (),
        _ => return Err("Expected wav, source, clip, track or mix scope".into()),
    }
    let mut events = Vec::new();
    let mut add = |id: &str, time: f64, value: f64| events.push((id.to_owned(), time, value));
    if let Some(p) = &preset {
        if !render && c.get("events").is_some() {
            return Err("Choose a preset or explicit events".into());
        }
        for t in list(p, "triggers") {
            add(
                t["control"].as_str().ok_or("Invalid trigger")?,
                num(t, "time"),
                0.,
            );
        }
        for lane in list(p, "lanes") {
            let id = lane["control"].as_str().ok_or("Invalid lane")?;
            let points = list(lane, "points");
            let first = points.first().ok_or("Empty automation lane")?;
            add(id, 0., num(first, "value"));
            for pair in points.windows(2) {
                let t = num(&pair[0], "time");
                let v = num(&pair[0], "value");
                let span = num(&pair[1], "time") - t;
                let delta = num(&pair[1], "value") - v;
                let steps = (span / 0.005).ceil() as usize;
                for step in 1..steps {
                    let f = step as f64 / steps as f64;
                    add(id, t + span * f, v + delta * f)
                }
                add(id, num(&pair[1], "time"), num(&pair[1], "value"));
            }
        }
        metadata["preset"] = c["preset"].clone();
    } else if !render {
        if c.get("events").is_some() && !c["events"].is_array() {
            return Err("events must be an array".into());
        }
        for e in list(c, "events") {
            let control = find(&asset, "controls", &e["control"])?;
            let value = if control["type"] == "slider" {
                number(
                    e,
                    "value",
                    f64::NAN,
                    num(control, "min"),
                    num(control, "max"),
                )?
            } else {
                0.
            };
            add(
                control["id"].as_str().ok_or("Invalid control")?,
                number(e, "time", f64::NAN, 0., 120.)?,
                value,
            )
        }
    }
    if let Some(id) = trigger
        .as_str()
        .filter(|id| !events.iter().any(|e| e.0 == *id))
    {
        events.insert(0, (id.to_owned(), 0., 0.));
    }
    let mut length = 0f64;
    let mut sources = HashMap::new();
    for t in list(&asset, "tracks") {
        for clip in list(t, "clips") {
            let id = clip["source"].as_str().ok_or("Missing source")?;
            if !sources.contains_key(id) {
                sources.insert(
                    id.to_owned(),
                    load(find(&asset, "sources", &clip["source"])?)?,
                );
            }
            if clip["trigger"].is_null() {
                length = length.max(num(clip, "start") + num(clip, "duration"))
            } else if !render {
                for e in &events {
                    if clip["trigger"] == e.0 {
                        length = length.max(e.1 + num(clip, "duration"))
                    }
                }
            }
        }
    }
    if !render && c["sustain"] == true {
        length = length.max(num(c, "end"));
    }
    let duration = if let Some(p) = &preset {
        num(p, "seconds")
    } else {
        number(
            c,
            "seconds",
            if render { length } else { length.min(120.) },
            if render { f64::MIN_POSITIVE } else { 1. / RATE },
            120.,
        )?
    };
    let sustain = preset
        .as_ref()
        .and_then(|p| p["sustain"].as_bool())
        .unwrap_or(c["sustain"] == true);
    events.sort_by(|a, b| a.1.total_cmp(&b.1));
    let events = events
        .iter()
        .map(|(id, t, value)| Event {
            id,
            frame: (t * RATE).round() as usize,
            value: *value,
        })
        .collect::<Vec<_>>();
    let audio = mix(
        &asset,
        &sources,
        if render {
            duration
        } else {
            (duration * RATE).ceil() / RATE
        },
        &events,
        sustain,
    )?;
    if render {
        let mut peak = 0f32;
        let mut clipped = 0;
        for s in audio.channels.iter().flatten() {
            peak = peak.max(s.abs());
            if s.abs() > 1. {
                clipped += 1;
            }
        }
        return Ok((
            json!({"peak":peak as f64,"clippedSamples":clipped,"duration":audio.duration()}),
            encode(&audio),
        ));
    }
    analysis::analyze(&audio, c, metadata)
}
