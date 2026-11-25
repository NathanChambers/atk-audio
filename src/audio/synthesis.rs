use super::*;
use std::f64::consts::TAU;
fn random(state: &mut u32) -> f64 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    f64::from(*state) / 2147483648. - 1.
}
struct Partial {
    phase: f64,
    step: f64,
    sweep: f64,
    amplitude: f64,
    damping: f64,
}
pub(super) fn generate(c: &Value) -> Result<(Audio, Value)> {
    if c.get("instrument").is_none() && c["wave"] == "formant" {
        return formants(c);
    }
    let instrument = c.get("instrument");
    let mut wave = c["wave"].as_str().unwrap_or("tone");
    let mut definitions = c["partials"].clone();
    let mut default_frequency = 440.;
    let mut default_attack = 0.005;
    let strike = if instrument.is_some() {
        number(c, "strike", 0.5, 0., 1.)?
    } else {
        0.
    };
    let seconds = number(
        c,
        "seconds",
        if instrument.is_some() { 1.5 } else { 1. },
        0.001,
        120.,
    )?;
    if let Some(name) = instrument {
        let (hz, attack, parts): (f64, f64, &[(f64, f64, f64)]) = match name.as_str() {
            Some("muted-pluck") => (
                440.,
                0.008,
                &[
                    (1., 1., 0.42),
                    (2., 0.53, 0.22),
                    (3., 0.29, 0.11),
                    (5., 0.12, 0.055),
                ],
            ),
            Some("rounded-bass") => (73.416, 0.018, &[(1., 1., 0.60), (2., 0.11, 0.24)]),
            Some("ceramic-tap") => (
                880.,
                0.008,
                &[
                    (1., 1., 0.24),
                    (2.72, 0.54, 0.15),
                    (4.18, 0.26, 0.08),
                    (6.43, 0.10, 0.04),
                ],
            ),
            _ => return Err(format!("Unknown instrument: {name}").into()),
        };
        if c.get("wave").is_some() || c.get("partials").is_some() {
            return Err("Choose either an instrument or explicit wave settings".into());
        }
        default_frequency = hz;
        default_attack = attack * (1. - 0.625 * strike);
        wave = "additive";
        let frequency = number(c, "frequency", hz, 1., 20000.)?;
        let end = number(c, "endFrequency", frequency, 1., 20000.)?;
        let mut partials = Vec::new();
        for &(ratio, weight, decay) in parts {
            if frequency.max(end) * ratio > 20000. {
                continue;
            }
            let gain = if ratio == 1. {
                weight
            } else {
                weight * (0.53 + 0.47 * strike)
            };
            partials.push(json!({
                "frequency": frequency * ratio,
                "endFrequency": end * ratio,
                "gain": gain,
                "decay": decay,
            }));
        }
        definitions = Value::Array(partials);
    }
    if !matches!(wave, "additive" | "noise" | "tone")
        || (!c["wave"].is_null() && !c["wave"].is_string())
    {
        return Err("Wave must be tone, noise, additive or formant".into());
    }
    let frequency = number(c, "frequency", default_frequency, 1., 20000.)?;
    let end = number(c, "endFrequency", frequency, 1., 20000.)?;
    let attack = number(c, "attack", default_attack.min(seconds), 0., seconds)?;
    let release = number(
        c,
        "release",
        seconds.min(if instrument.is_some() { 0.1 } else { 0.005 }),
        0.,
        seconds,
    )?;
    let gain = number(c, "gain", 0.7, 0., 1.)?;
    let harmonics = number(c, "harmonics", 1., 1., 32.)? as usize;
    let mut seed = number(c, "seed", 1., 0., u32::MAX as f64)? as u32;
    let mut settings = json!({"wave":wave,"seconds":seconds,"frequency":frequency,"endFrequency":end,"attack":attack,"release":release,"gain":gain,"seed":seed,"harmonics":harmonics});
    if let Some(name) = instrument {
        settings["instrument"] = name.clone();
        settings["strike"] = json!(strike)
    }
    let mut partials = Vec::new();
    if wave == "additive" {
        let entries = definitions
            .as_array()
            .filter(|a| (1..=32).contains(&a.len()))
            .ok_or("Additive synthesis requires 1 to 32 partials")?;
        let mut resolved = Vec::new();
        for p in entries {
            if !p.is_object() {
                return Err("Expected a partial object".into());
            }
            let hz = number(p, "frequency", f64::NAN, 1., 20000.)?;
            let end = number(p, "endFrequency", hz, 1., 20000.)?;
            let gain = number(p, "gain", 1., 0., 1.)?;
            let phase = number(p, "phaseDegrees", 0., -360., 360.)?;
            let decay = if p.get("decay").is_some() {
                Some(number(p, "decay", f64::NAN, 0.001, 120.)?)
            } else {
                None
            };
            partials.push(Partial {
                phase: phase * std::f64::consts::PI / 180.,
                step: TAU * hz / RATE,
                sweep: TAU * (end - hz) / (RATE * RATE * seconds),
                amplitude: gain,
                damping: decay.map_or(1., |d| (-1. / (d * RATE)).exp()),
            });
            resolved.push(json!({"frequency":hz,"endFrequency":end,"gain":gain,"phaseDegrees":phase,"decay":decay}));
        }
        settings["partials"] = json!(resolved);
    }
    let am = if let Some(am) = c.get("am") {
        let hz = number(am, "frequency", f64::NAN, 0.01, 1000.)?;
        let depth = number(am, "depth", f64::NAN, 0., 1.)?;
        settings["am"] = json!({"frequency":hz,"depth":depth});
        Some((hz, depth))
    } else {
        None
    };
    let harmonic_gain = (1..=harmonics).map(|h| 1. / (h * h) as f64).sum::<f64>();
    let partial_gain = partials.iter().map(|p| p.amplitude).sum::<f64>().max(1.);
    let frames = (seconds * RATE).round() as usize;
    let mut samples = Vec::with_capacity(frames);
    let mut phase = 0f64;
    let mut filtered = 0.;
    for i in 0..frames {
        let time = i as f64 / RATE;
        let hz = frequency + (end - frequency) * time / seconds;
        let mut value = match wave {
            "noise" => {
                filtered += (1. - (-TAU * hz / RATE).exp()) * (random(&mut seed) - filtered);
                filtered
            }
            "additive" => {
                let mut sum = 0.;
                for p in &mut partials {
                    sum += p.amplitude * p.phase.sin();
                    p.phase = (p.phase + p.step) % TAU;
                    p.step += p.sweep;
                    p.amplitude *= p.damping;
                }
                sum / partial_gain
            }
            _ => {
                let mut sum = 0.;
                for h in 1..=harmonics {
                    if hz * h as f64 >= RATE / 2. {
                        break;
                    }
                    sum += (phase * h as f64).sin() / (h * h) as f64;
                }
                phase = (phase + TAU * hz / RATE) % TAU;
                sum / harmonic_gain
            }
        };
        if let Some((hz, depth)) = am {
            value *= 1. - depth * (1. - (TAU * hz * time).cos()) / 2.;
        }
        let fade = if attack > 0. {
            (time / attack).min(1.)
        } else {
            1.
        };
        let fade = if release > 0. {
            fade.min((frames - 1 - i) as f64 / (RATE * release))
        } else {
            fade
        };
        samples.push((value * gain * fade) as f32);
    }
    Ok((
        Audio {
            channels: vec![samples],
            rate: RATE,
        },
        settings,
    ))
}
struct Segment {
    hz: f64,
    end: f64,
    gain: f64,
    noise: f64,
    bands: Vec<f64>,
    frames: usize,
}
impl Segment {
    fn level(&self, hz: f64) -> f64 {
        self.bands
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let d = (hz - f) / (90. + i as f64 * 60.);
                1. / ((1. + i as f64 * 0.6) * (1. + d * d))
            })
            .sum()
    }
}
fn formants(c: &Value) -> Result<(Audio, Value)> {
    let entries = list(c, "segments");
    if !(1..=1000).contains(&entries.len()) {
        return Err("Formant synthesis requires 1 to 1000 segments".into());
    }
    let transition = number(c, "transition", 0., 0., 0.06)?;
    let glide_frames = (transition * RATE).round() as usize;
    let gain = number(c, "gain", 0.7, 0., 1.)?;
    let mut seed = number(c, "seed", 1., 0., u32::MAX as f64)? as u32;
    let mut settings = json!({"wave":"formant","gain":gain,"seed":seed,"transition":transition});
    let mut segments = Vec::new();
    let mut resolved = Vec::new();
    let mut total = 0;
    for entry in entries {
        let seconds = number(entry, "seconds", f64::NAN, 0.001, 120.)?;
        let hz = number(entry, "frequency", 135., 40., 500.)?;
        let end = number(entry, "endFrequency", hz, 40., 500.)?;
        let gain = number(entry, "gain", 1., 0., 1.)?;
        let noise = number(entry, "noise", 0., 0., 1.)?;
        let bands = list(entry, "formants");
        if (!entry["formants"].is_null() && !entry["formants"].is_array()) || bands.len() > 5 {
            return Err("Expected up to five formants".into());
        }
        let bands = bands
            .iter()
            .map(|v| {
                v.as_f64()
                    .filter(|n| (80.0..=6000.).contains(n))
                    .ok_or("Invalid formant")
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let frames = (seconds * RATE).round() as usize;
        total += frames;
        if total > 120 * 48000 {
            return Err("Invalid total segment duration".into());
        }
        resolved.push(json!({"seconds":seconds,"frequency":hz,"endFrequency":end,"gain":gain,"noise":noise,"formants":bands}));
        segments.push(Segment {
            hz,
            end,
            gain,
            noise,
            bands,
            frames,
        });
    }
    settings["segments"] = json!(resolved);
    let mut samples = Vec::with_capacity(total);
    let mut phase = 0.;
    let mut previous_noise = 0.;
    let mut peak = 0f64;
    for (index, s) in segments.iter().enumerate() {
        let previous = index
            .checked_sub(1)
            .filter(|_| glide_frames > 0)
            .map(|i| &segments[i]);
        let glide = glide_frames.min(s.frames / 2);
        let mut levels = vec![
            0.;
            if !s.bands.is_empty() || previous.is_some_and(|p| !p.bands.is_empty()) {
                48
            } else {
                0
            }
        ];
        for i in 0..s.frames {
            let blend = if previous.is_some() && glide > 0 {
                (i as f64 / glide as f64).min(1.)
            } else {
                1.
            };
            let interp = |from: f64, to: f64| {
                if blend < 1. {
                    from + (to - from) * blend
                } else {
                    to
                }
            };
            let hz = interp(
                previous.map_or(0., |p| p.end),
                s.hz + (s.end - s.hz) * i as f64 / s.frames as f64,
            );
            if i % 64 == 0 {
                for (h, level) in levels.iter_mut().enumerate() {
                    let harmonic = (h + 1) as f64;
                    let f = harmonic * hz;
                    *level = (s.level(f) * blend
                        + previous.map_or(0., |p| p.level(f) * (1. - blend)))
                        / harmonic.sqrt();
                }
            }
            phase = (phase + TAU * hz / RATE) % TAU;
            let mut value = levels
                .iter()
                .enumerate()
                .map(|(i, l)| l * ((i + 1) as f64 * phase).sin())
                .sum::<f64>();
            let noise = interp(previous.map_or(0., |p| p.noise), s.noise);
            if noise > 0. {
                let current = random(&mut seed);
                value += noise * (current - 0.7 * previous_noise);
                previous_noise = current;
            }
            let attack = if previous.is_some() {
                1.
            } else {
                i as f64 / 240.
            };
            let release = if glide_frames > 0 && index + 1 < segments.len() {
                1.
            } else {
                (s.frames - 1 - i) as f64 / 240.
            };
            value *= interp(previous.map_or(0., |p| p.gain), s.gain) * attack.min(release).min(1.);
            peak = peak.max(value.abs());
            samples.push(value as f32);
        }
    }
    if peak > 0. {
        let scale = gain / peak;
        for x in &mut samples {
            *x = (f64::from(*x) * scale) as f32
        }
    }
    Ok((
        Audio {
            channels: vec![samples],
            rate: RATE,
        },
        settings,
    ))
}
