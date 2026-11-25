use super::*;
use rustfft::{Fft, FftPlanner, num_complex::Complex64};
use std::sync::Arc;
fn round(v: f64, d: i32) -> f64 {
    let factor = 10f64.powi(d);
    (v * factor + 0.5).floor() / factor
}
fn db(p: f64) -> f64 {
    round((10. * p.log10()).max(-120.), 2)
}
fn levels(audio: &Audio, first: usize, last: usize) -> (f64, f64) {
    let mut peak = 0f64;
    let mut power = 0.;
    for ch in &audio.channels {
        for &x in &ch[first..last] {
            let s = x as f64;
            peak = peak.max(s.abs());
            power += s * s;
        }
    }
    (
        db(peak * peak),
        db(power / ((last - first) * audio.channels.len()) as f64),
    )
}
pub(super) struct Spectrum {
    fft: Arc<dyn Fft<f64>>,
    window: Vec<f64>,
    normal: f64,
    rms: bool,
    buffer: Vec<Complex64>,
    scratch: Vec<Complex64>,
}
impl Spectrum {
    pub(super) fn new(size: usize, rms: bool, channels: usize) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(size);
        let window: Vec<_> = (0..size)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / size as f64).cos())
            .collect();
        let norm = if rms {
            size as f64 * window.iter().map(|v| v * v).sum::<f64>()
        } else {
            window.iter().sum::<f64>().powi(2)
        };
        let scratch = vec![Complex64::default(); fft.get_inplace_scratch_len()];
        Self {
            fft,
            window,
            normal: 1. / (norm * channels as f64),
            rms,
            buffer: vec![Complex64::default(); size],
            scratch,
        }
    }
    pub(super) fn powers(&mut self, a: &Audio, center: usize) -> Vec<f64> {
        let n = self.window.len();
        let mut p = vec![0.; n / 2 + 1];
        for ch in &a.channels {
            for (i, x) in self.buffer.iter_mut().enumerate() {
                let sample = (center + i)
                    .checked_sub(n / 2)
                    .and_then(|j| ch.get(j))
                    .copied()
                    .unwrap_or(0.);
                *x = Complex64::new(sample as f64 * self.window[i], 0.);
            }
            self.fft
                .process_with_scratch(&mut self.buffer, &mut self.scratch);
            for (i, v) in p.iter_mut().enumerate() {
                let weight = if i == 0 || i == n / 2 {
                    1.
                } else if self.rms {
                    2.
                } else {
                    4.
                };
                *v += weight * self.buffer[i].norm_sqr() * self.normal;
            }
        }
        p
    }
}
fn corr(pairs: impl Iterator<Item = (f64, f64)>) -> Option<f64> {
    let (mut ab, mut aa, mut bb) = (0., 0., 0.);
    for (x, y) in pairs {
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    (aa > 0. && bb > 0.).then(|| (ab / (aa * bb).sqrt()).clamp(-1., 1.))
}
fn pitch(a: &Audio, first: usize, last: usize, center: usize) -> Option<(f64, f64)> {
    let stride = ((a.rate / 6000.) as usize).max(1);
    let rate = a.rate / stride as f64;
    let min = ((rate / 1000.) as usize).max(2);
    let max = (rate / 25.).ceil() as usize;
    let count = ((rate * 0.04).round() as usize + max).min((last - first) / stride);
    if count < min * 3 {
        return None;
    }
    let start = center
        .saturating_sub(count * stride / 2)
        .clamp(first, last - count * stride);
    let mut scores = vec![0f64; (max + 2).min(count / 2)];
    for ch in &a.channels {
        let mut vals: Vec<_> = ch[start..start + count * stride]
            .chunks_exact(stride)
            .map(|chunk| chunk.iter().map(|&x| x as f64).sum::<f64>() / stride as f64)
            .collect();
        let mean = vals.iter().map(|n| n / count as f64).sum::<f64>();
        for n in &mut vals {
            *n -= mean;
        }
        for (lag, s) in scores.iter_mut().enumerate().skip(min - 1) {
            *s = s.max(corr(vals.iter().copied().zip(vals[lag..].iter().copied())).unwrap_or(0.));
        }
    }
    let threshold = (scores.iter().copied().fold(0., f64::max) * 0.94).max(0.7);
    for lag in min..scores.len().saturating_sub(1) {
        let (l, m, r) = (scores[lag - 1], scores[lag], scores[lag + 1]);
        if m >= threshold && m > l && m >= r {
            let hz = rate / (lag as f64 + 0.5 * (l - r) / (l - 2. * m + r));
            if (25.0..=1000.).contains(&hz) {
                return Some((round(hz, 2), round(m, 3)));
            }
        }
    }
    None
}
pub(super) fn melody(a: &Audio) -> Result<Vec<Value>> {
    let hop = (a.rate * 0.01).round() as usize;
    let win = (a.rate * 0.025).round() as usize;
    let n = a.channels[0].len();
    let pitches: Vec<_> = (0..n)
        .step_by(hop)
        .map(|i| {
            let center = i + hop / 2;
            if levels(a, i, (i + hop).min(n)).1 <= -50. {
                return None;
            }
            pitch(a, center.saturating_sub(win), (center + win).min(n), center)
                .map(|(hz, _)| (69. + 12. * (hz / 440.).log2()).round() as i32)
        })
        .collect();
    let mut stable = pitches.clone();
    for i in 1..pitches.len().saturating_sub(1) {
        if pitches[i - 1].is_some() && pitches[i - 1] == pitches[i + 1] {
            stable[i] = pitches[i - 1]
        }
    }
    let mut notes = Vec::new();
    let mut i = 0;
    let mut offset = None;
    while i < stable.len() {
        let first = i;
        let midi = stable[i];
        i += 1;
        while i < stable.len() && stable[i] == midi {
            i += 1
        }
        if midi.is_some() && i - first >= 5 {
            let start = (first * hop) as f64 / a.rate;
            notes.push(json!({"midi":midi,"start":round(start-*offset.get_or_insert(start),6),"duration":round((i*hop).min(n) as f64/a.rate-start,6)}))
        }
    }
    if notes.is_empty() {
        return Err(
            "No clear sung notes found. Try a single, steady melody in a quiet room.".into(),
        );
    }
    Ok(notes)
}
fn modulation(values: &[Option<f64>], hop: f64, unit: &str) -> Value {
    let mut sorted: Vec<_> = values.iter().flatten().copied().collect();
    sorted.sort_by(f64::total_cmp);
    if sorted.len() < 2 {
        return json!({"depth":null,"unit":unit,"rateHz":null});
    }
    let n = (sorted.len() - 1) as f64;
    let depth = sorted[(n * 0.95) as usize] - sorted[(n * 0.05) as usize];
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let max = (values.len() / 3).min((2. / hop).ceil() as usize);
    let mut scores = vec![0.; max + 2];
    for (lag, score) in scores.iter_mut().enumerate().skip(1) {
        let pairs: Vec<_> = values
            .iter()
            .zip(values.iter().skip(lag))
            .filter_map(|(a, b)| Some(((*a)? - mean, (*b)? - mean)))
            .collect();
        if pairs.len() >= 8.min(values.len().saturating_sub(lag)) {
            *score = corr(pairs.into_iter()).unwrap_or(0.);
        }
    }
    let mut valley = false;
    let mut hz = None;
    for lag in 2..=max {
        let (l, m, r) = (scores[lag - 1], scores[lag], scores[lag + 1]);
        valley |= m < 0.;
        if valley && lag >= 3 && m >= 0.6 && m > l && m >= r {
            hz = Some(round(
                1. / ((lag as f64 + 0.5 * (l - r) / (l - 2. * m + r)) * hop),
                2,
            ));
            break;
        }
    }
    json!({"depth":round(depth,2),"unit":unit,"rateHz":hz})
}
fn onsets(a: &Audio, first: usize, last: usize) -> Vec<Value> {
    let hop = (a.rate * 0.002).round() as usize;
    let env: Vec<_> = (first..last)
        .step_by(hop)
        .map(|i| levels(a, i, (i + hop).min(last)).1)
        .collect();
    let floor = (env.iter().copied().fold(-120., f64::max) - 40.).max(-65.);
    let mut out = Vec::new();
    let mut next = 0;
    for (i, &v) in env.iter().enumerate() {
        let base = i.saturating_sub(5);
        if i < next || v < floor || (i > 0 && v < env[base] + 8.) {
            continue;
        }
        let mut peak = i;
        for j in i + 1..(i + 25).min(env.len()) {
            if env[j] > env[peak] {
                peak = j
            }
        }
        let mut start = i;
        while start > base && env[start - 1] > floor.max(env[peak] - 20.) {
            start -= 1;
        }
        let mut end = start;
        while end < peak && env[end] < env[peak] - 1. {
            end += 1;
        }
        out.push(json!({"time":round((first+start*hop) as f64/a.rate,6),"attackMs":round(((end-start)*hop) as f64*1000./a.rate,2),"peakRmsDbfs":env[peak]}));
        next = peak + 25;
    }
    out
}
fn stereo(a: &Audio, first: usize, last: usize) -> Value {
    if a.channels.len() < 2 {
        return Value::Null;
    }
    let n = last - first;
    let left = &a.channels[0][first..last];
    let right = &a.channels[1][first..last];
    let score = |lag: isize, stride: usize| {
        let l = (-lag).max(0) as usize;
        let r = lag.max(0) as usize;
        corr(
            (0..n - lag.unsigned_abs())
                .step_by(stride)
                .map(|i| (left[l + i] as f64, right[r + i] as f64)),
        )
    };
    let zero = score(0, 1);
    let mut best = zero.unwrap_or(0.).abs();
    let mut delay = 0;
    if zero.is_some() && best < 0.999 {
        let limit = ((a.rate * 0.02).round() as usize).min(n / 4) as isize;
        for lag in -limit..=limit {
            let s = score(lag, (n / 2048).max(1)).unwrap_or(0.).abs();
            if s > best + 0.001 {
                delay = lag;
                best = s;
            }
        }
    }
    let (mut mid, mut side) = (0., 0.);
    for (&l, &r) in left.iter().zip(right) {
        mid += (l as f64 + r as f64).powi(2);
        side += (l as f64 - r as f64).powi(2);
    }
    json!({"correlation":zero.map(|z|round(z,4)),"sideEnergyFraction":(mid+side>0.).then(||round(side/(mid+side),4)),"delayMs":(best>=0.6).then(||round(delay as f64*1000./a.rate,4)),"delayConfidence":(best>=0.6).then(||round(best,3))})
}
fn impulse(a: &Audio, first: usize, last: usize) -> Value {
    let mut peak = 0f64;
    let mut arrival = first;
    for i in first..last {
        for ch in &a.channels {
            let x = (ch[i] as f64).abs();
            if x > peak {
                arrival = i;
                peak = x
            }
        }
    }
    let mut energy = vec![0.; last - arrival];
    let mut sum = 0.;
    for i in (arrival..last).rev() {
        for ch in &a.channels {
            sum += f64::from(ch[i]).powi(2);
        }
        energy[i - arrival] = sum;
    }
    let crossing = |db: f64| energy.iter().position(|x| *x <= sum * 10f64.powf(db / 10.));
    let decay = |db, factor| {
        let start = crossing(-5.)?;
        let end = crossing(db)?;
        (end > start).then(|| round((end - start) as f64 * factor / a.rate, 4))
    };
    json!({"peakTime":(peak>0.).then(||round(arrival as f64/a.rate,6)),"peakDbfs":db(peak*peak),"t20Seconds":decay(-25.,3.),"t30Seconds":decay(-35.,2.)})
}
pub(super) fn analyze(a: &Audio, c: &Value, mut meta: Value) -> Result<(Value, Vec<u8>)> {
    let image = c["command"] == "spectrogram";
    let format = if image {
        "spectrogram"
    } else {
        option_str(c, "format", "features")?
    };
    let start = number(c, "start", 0., 0., a.duration())?;
    let end = number(c, "end", a.duration(), start, a.duration())?;
    if end <= start {
        return Err("end must be after start".into());
    }
    let first = ((start * a.rate).round() as usize).min(a.channels[0].len() - 1);
    let last = ((end * a.rate).round() as usize).max(first + 1);
    meta["sampleRate"] = json!(a.rate as u32);
    if !image {
        meta["format"] = json!(format);
        meta["channels"] = json!(a.channels.len());
    }
    meta["start"] = json!(first as f64 / a.rate);
    meta["end"] = json!(last as f64 / a.rate);
    if !image {
        match format {
            "pcm" => {
                if last - first > 8192 {
                    return Err("PCM output supports up to 8192 frames. Use start and end to select a short range.".into());
                }
                meta["firstFrame"] = json!(first);
                meta["frames"] = json!(last - first);
                meta["samples"] = json!(
                    a.channels
                        .iter()
                        .map(|ch| ch[first..last]
                            .iter()
                            .map(|&x| x as f64)
                            .collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                );
                return Ok((meta, vec![]));
            }
            "impulse" => {
                meta.as_object_mut()
                    .unwrap()
                    .extend(impulse(a, first, last).as_object().unwrap().clone());
                return Ok((meta, vec![]));
            }
            "waveform" => {
                meta["start"] = json!(start);
                meta["end"] = json!(end);
                return plots::waveform(a, meta, first, last);
            }
            "features" | "mel" => (),
            _ => return Err("Expected waveform, features, mel, pcm or impulse format".into()),
        }
    }
    let size = number(c, "fftSize", 4096., 512., 16384.)?;
    if size.fract() != 0. || !(size as usize).is_power_of_two() {
        return Err("fftSize must be a power of two from 512 to 16384".into());
    }
    let size = size as usize;
    meta["fftSize"] = json!(size);
    if image {
        meta["start"] = json!(start);
        meta["end"] = json!(end);
        return plots::spectrogram(a, c, meta, size);
    }
    let bin = a.rate / size as f64;
    let max = number(c, "maxHz", 16000f64.min(a.rate / 2.), bin, a.rate / 2.)?;
    let hop = (number(c, "hopMs", 50., 10., 1000.)? * a.rate / 1000.).round() as usize;
    if (last - first).div_ceil(hop) > 3000 {
        return Err("Analysis supports up to 3000 windows".into());
    }
    let hop_sec = hop as f64 / a.rate;
    meta["windowSeconds"] = json!(size as f64 / a.rate);
    meta["hopSeconds"] = json!(hop_sec);
    meta["maxHz"] = json!(max);
    meta["dbFloor"] = json!(-120);
    let mut spectrum = Spectrum::new(size, true, a.channels.len());
    if format == "mel" {
        let bands = number(c, "melBands", 40., 8., 96.)?;
        if bands.fract() != 0. {
            return Err("melBands must be an integer from 8 to 96".into());
        }
        let min = number(c, "minHz", 0., 0., max)?;
        if min >= max {
            return Err("minHz must be below maxHz".into());
        }
        let bands = bands as usize;
        let base = (min / 700.).ln_1p();
        let step = ((max / 700.).ln_1p() - base) / (bands + 1) as f64;
        let hz = |i: usize| 700. * (base + i as f64 * step).exp_m1();
        let mut weights = Vec::new();
        for i in 0..bands {
            let (l, m, h) = (hz(i), hz(i + 1), hz(i + 2));
            let entries: Vec<_> = ((l / bin).ceil() as usize..=(h / bin).floor() as usize)
                .map(|k| {
                    let f = k as f64 * bin;
                    (k, ((f - l) / (m - l)).min((h - f) / (h - m)).max(0.))
                })
                .collect();
            weights.push(entries);
        }
        let mut times = Vec::new();
        let mut values = Vec::new();
        for i in (first..last).step_by(hop) {
            let center = (i + (i + hop).min(last)) / 2;
            let p = spectrum.powers(a, center);
            times.push(center as f64 / a.rate);
            values.push(
                weights
                    .iter()
                    .map(|band| db(band.iter().map(|&(k, w)| p[k] * w).sum()))
                    .collect::<Vec<_>>(),
            );
        }
        meta["minHz"] = json!(min);
        meta["bandsHz"] = json!((1..=bands).map(|i| round(hz(i), 1)).collect::<Vec<_>>());
        meta["times"] = json!(times);
        meta["valuesDbfs"] = json!(values);
        return Ok((meta, vec![]));
    }
    let mut edges = vec![0., 60., 250., 500., 2000., 4000., 8000., 16000.];
    edges.retain(|&f| f < max);
    edges.push(max);
    meta["bands"] = json!(
        edges
            .windows(2)
            .map(|w| json!({"lowHz":w[0],"highHz":w[1]}))
            .collect::<Vec<_>>()
    );
    let last_bin = (max / bin).floor() as usize;
    let mut previous = vec![0.; last_bin + 1];
    let mut frames = Vec::new();
    let (mut pitches, mut levels_trace, mut timbres) = (Vec::new(), Vec::new(), Vec::new());
    for i in (first..last).step_by(hop) {
        let stop = (i + hop).min(last);
        let center = (i + stop) / 2;
        let p = spectrum.powers(a, center);
        let powers = &p[..=last_bin];
        let mags = powers.iter().map(|p| p.sqrt()).collect::<Vec<_>>();
        let sum = mags.iter().sum::<f64>();
        let power = powers.iter().sum::<f64>();
        let mut bands = vec![0.; edges.len() - 1];
        let mut flux = 0.;
        let mut weighted = 0.;
        let mut logs = 0.;
        for (k, &m) in mags.iter().enumerate() {
            let hz = k as f64 * bin;
            let band = edges
                .partition_point(|&edge| edge <= hz)
                .saturating_sub(1)
                .min(edges.len() - 2);
            bands[band] += powers[k];
            weighted += m * hz;
            logs += powers[k].max(1e-30).ln();
            let normalized = if sum > 0. { m / sum } else { 0. };
            flux += (normalized - previous[k]).max(0.).powi(2);
            previous[k] = normalized;
        }
        let centroid = (sum > 0.).then(|| round(weighted / sum, 1));
        let n = powers.len() as f64;
        let flat = (power > 0.).then(|| round(((logs / n).exp() / (power / n)).min(1.), 6));
        let (peak, rms) = levels(a, i, stop);
        let pitch = if rms > -65. {
            pitch(a, first, last, center)
        } else {
            None
        };
        let info=pitch.map(|(hz,confidence)|{let mut harmonics=Vec::new();
            if hz>=bin*3.{for order in 1..=8{let k=(order as f64*hz/bin).round() as usize;if k>last_bin{break}let mut best=k;for j in k-1..=(k+1).min(last_bin){if powers[j]>powers[best]{best=j}}harmonics.push(json!({"order":order,"frequencyHz":round(best as f64*bin,2),"peakDbfs":db(powers[best])}));}}
            json!({"fundamentalHz":hz,"confidence":confidence,"harmonics":harmonics})});
        frames.push(json!({"time":center as f64/a.rate,"firstFrame":i,"frameCount":stop-i,"peakDbfs":peak,"rmsDbfs":rms,"bandDbfs":bands.into_iter().map(db).collect::<Vec<_>>(),"centroidHz":centroid,"flatness":flat,"flux":(i!=first).then(||round(flux.sqrt(),6)),"pitch":info}));
        pitches.push(pitch.map(|(hz, _)| 1200. * (hz / 440.).log2()));
        levels_trace.push((rms > -65.).then_some(rms));
        timbres.push(centroid.filter(|_| rms > -65.));
    }
    meta["frames"] = json!(frames);
    meta["onsets"] = json!(onsets(a, first, last));
    meta["stereo"] = stereo(a, first, last);
    meta["modulation"] = json!({"pitch":modulation(&pitches,hop_sec,"cents"),"level":modulation(&levels_trace,hop_sec,"dB"),"timbre":modulation(&timbres,hop_sec,"Hz")});
    Ok((meta, vec![]))
}
