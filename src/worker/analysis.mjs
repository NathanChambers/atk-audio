import {analysisFftSize, analysisRange, createPowerSpectrum, createWaveform} from "./spectrogram.mjs";

const bandEdges = [0, 60, 250, 500, 2000, 4000, 8000, 16000];
const dbFloor = -120;

export function analyzeAudio(audio, options) {
    const format = options.format ?? "features";
    if (format === "waveform") return {format, ...createWaveform(audio, options)};
    if (!["features", "mel", "pcm", "impulse"].includes(format)) throw new Error("Expected waveform, features, mel, pcm or impulse format");

    const range = analysisRange(audio, options);
    const metadata = {
        format, sampleRate: audio.rate, channels: audio.samples.length,
        start: range.first / audio.rate, end: range.last / audio.rate,
    };
    if (format === "impulse") return {...metadata, ...measureImpulse(audio, range)};

    if (format === "pcm") {
        const frames = range.last - range.first;
        if (frames > 8192) throw new Error("PCM output supports up to 8192 frames. Use start and end to select a short range.");

        const samples = audio.samples.map(channel => Array.from(channel.subarray(range.first, range.last)));
        return {...metadata, firstFrame: range.first, frames, samples};
    }

    const fftSize = analysisFftSize(options);
    const hopMs = options.hopMs ?? 50;
    if (!Number.isFinite(hopMs) || hopMs < 10 || hopMs > 1000) throw new Error("hopMs must be between 10 and 1000");

    const hopFrames = Math.round(hopMs * audio.rate / 1000);
    const frameCount = Math.ceil((range.last - range.first) / hopFrames);
    if (frameCount > 3000) throw new Error("Analysis supports up to 3000 windows. Increase hopMs or select a shorter range.");

    const binHz = audio.rate / fftSize;
    const maxHz = options.maxHz ?? Math.min(16000, audio.rate / 2);
    if (!Number.isFinite(maxHz) || maxHz < binHz || maxHz > audio.rate / 2) throw new Error("maxHz must be within the measurable frequency range");

    const grid = {fftSize, windowSeconds: fftSize / audio.rate, hopSeconds: hopFrames / audio.rate, maxHz, dbFloor};
    const spectrum = createPowerSpectrum(audio.samples, fftSize, true);
    if (format === "mel") return {...metadata, ...grid, ...measureMel(spectrum, audio.rate, range, hopFrames, binHz, options, maxHz)};
    const features = measureFeatures(audio, spectrum, range, hopFrames, binHz, maxHz);
    return {
        ...metadata, ...grid, ...features,
        onsets: measureOnsets(audio, range),
        modulation: measureModulation(features.frames, grid.hopSeconds),
        stereo: measureStereo(audio, range),
    };
}

function measureFeatures(audio, spectrum, range, hop, binHz, maxHz) {
    const edges = bandEdges.filter(edge => edge < maxHz);
    edges.push(maxHz);
    const bands = edges.slice(1).map((highHz, index) => ({lowHz: edges[index], highHz}));
    const lastBin = Math.floor(maxHz / binHz);
    const binBands = new Uint8Array(lastBin + 1);
    for (let bin = 0, band = 0; bin <= lastBin; bin++) {
        while (band < bands.length - 1 && bin * binHz >= bands[band].highHz) band++;
        binBands[bin] = band;
    }

    const frames = [];
    const previous = new Float64Array(lastBin + 1);
    const magnitudes = new Float64Array(lastBin + 1);
    const powersByBand = new Float64Array(bands.length);
    for (let first = range.first; first < range.last; first += hop) {
        const last = Math.min(first + hop, range.last);
        const center = Math.floor((first + last) / 2);
        const levels = measureLevels(audio.samples, first, last);
        const powers = spectrum(center);
        powersByBand.fill(0);
        let magnitudeSum = 0;
        let weightedSum = 0;
        let powerSum = 0;
        let logSum = 0;
        for (let bin = 0; bin <= lastBin; bin++) {
            const power = powers[bin];
            const magnitude = Math.sqrt(power);
            magnitudes[bin] = magnitude;
            magnitudeSum += magnitude;
            weightedSum += magnitude * bin * binHz;
            powerSum += power;
            logSum += Math.log(Math.max(1e-30, power));
            powersByBand[binBands[bin]] += power;
        }

        let fluxSquared = 0;
        for (let bin = 0; bin <= lastBin; bin++) {
            const normalized = magnitudeSum > 0 ? magnitudes[bin] / magnitudeSum : 0;
            const increase = Math.max(0, normalized - previous[bin]);
            fluxSquared += increase ** 2;
            previous[bin] = normalized;
        }
        const centroid = magnitudeSum > 0 ? weightedSum / magnitudeSum : null;
        const flatness = powerSum > 0 ? Math.min(1, Math.exp(logSum / (lastBin + 1)) / (powerSum / (lastBin + 1))) : null;
        const pitch = levels.rmsDbfs > -65 ? measurePitch(audio, range, center) : null;
        if (pitch) pitch.harmonics = measureHarmonics(pitch.fundamentalHz, powers, binHz, lastBin);
        frames.push({
            time: center / audio.rate, firstFrame: first, frameCount: last - first,
            ...levels,
            bandDbfs: Array.from(powersByBand, toDb),
            centroidHz: centroid === null ? null : round(centroid, 1),
            flatness: flatness === null ? null : round(flatness, 6),
            flux: frames.length ? round(Math.sqrt(fluxSquared), 6) : null,
            pitch,
        });
    }

    return {bands, frames};
}

export function melodyNotes(audio) {
    const hop = Math.round(audio.rate * 0.01);
    const window = Math.round(audio.rate * 0.025);
    const frames = audio.samples[0].length;
    const pitches = [];
    for (let position = 0; position < frames; position += hop) {
        const center = position + Math.floor(hop / 2);
        const first = Math.max(0, center - window);
        const last = Math.min(frames, center + window);
        const levels = measureLevels(audio.samples, position, Math.min(position + hop, frames));
        const pitch = levels.rmsDbfs > -50 ? measurePitch(audio, {first, last}, center) : null;
        pitches.push(pitch ? Math.round(69 + 12 * Math.log2(pitch.fundamentalHz / 440)) : null);
    }

    const stable = pitches.map((pitch, index) => {
        const previous = pitches[index - 1];
        const next = pitches[index + 1];
        return previous != null && previous === next ? previous : pitch;
    });
    const notes = [];
    for (let first = 0; first < stable.length;) {
        const midi = stable[first];
        let last = first + 1;
        while (last < stable.length && stable[last] === midi) last++;

        if (midi !== null && last - first >= 5) {
            const start = first * hop / audio.rate;
            const end = Math.min(frames, last * hop) / audio.rate;
            notes.push({midi, start, duration: end - start});
        }
        first = last;
    }

    if (!notes.length) throw new Error("No clear sung notes found. Try a single, steady melody in a quiet room.");

    const offset = notes[0].start;
    for (const note of notes) {
        note.start = round(note.start - offset, 6);
        note.duration = round(note.duration, 6);
    }
    return notes;
}

function measurePitch(audio, range, center) {
    const stride = Math.max(1, Math.floor(audio.rate / 6000));
    const rate = audio.rate / stride;
    const minLag = Math.max(2, Math.floor(rate / 1000));
    const maxLag = Math.ceil(rate / 25);
    const span = Math.round(rate * 0.04);
    const count = Math.min(span + maxLag, Math.floor((range.last - range.first) / stride));
    if (count < minLag * 3) return null;

    const first = Math.max(range.first, Math.min(center - Math.floor(count * stride / 2), range.last - count * stride));
    const scores = new Float64Array(Math.min(maxLag + 2, Math.floor(count / 2)));
    let best = 0;
    for (const channel of audio.samples) {
        const samples = new Float64Array(count);
        let mean = 0;
        for (let index = 0; index < count; index++) {
            const offset = first + index * stride;
            let sum = 0;
            for (let frame = offset; frame < offset + stride; frame++) sum += channel[frame];
            samples[index] = sum / stride;
            mean += samples[index] / count;
        }
        for (let index = 0; index < count; index++) samples[index] -= mean;

        for (let lag = minLag - 1; lag < scores.length; lag++) {
            const score = correlation(samples, samples, 0, lag, count - lag) ?? 0;
            scores[lag] = Math.max(scores[lag], score);
            best = Math.max(best, score);
        }
    }

    for (let lag = minLag; lag < scores.length - 1; lag++) {
        const score = scores[lag];
        if (score < Math.max(0.7, best * 0.94) || score <= scores[lag - 1] || score < scores[lag + 1]) continue;

        const left = scores[lag - 1];
        const right = scores[lag + 1];
        const offset = 0.5 * (left - right) / (left - 2 * score + right);
        const frequency = rate / (lag + offset);
        if (frequency >= 25 && frequency <= 1000) {
            return {fundamentalHz: round(frequency, 2), confidence: round(score, 3)};
        }
    }
    return null;
}

function measureHarmonics(fundamental, powers, binHz, lastBin) {
    if (fundamental < binHz * 3) return [];

    const harmonics = [];
    for (let order = 1; order <= 8; order++) {
        const center = Math.round(order * fundamental / binHz);
        if (center > lastBin) break;

        let peak = center;
        for (let bin = center - 1; bin <= Math.min(lastBin, center + 1); bin++) {
            if (powers[bin] > powers[peak]) peak = bin;
        }
        harmonics.push({order, frequencyHz: round(peak * binHz, 2), peakDbfs: toDb(powers[peak])});
    }
    return harmonics;
}

function measureOnsets(audio, range) {
    const hop = Math.max(1, Math.round(audio.rate * 0.002));
    const envelope = [];
    for (let frame = range.first; frame < range.last; frame += hop) {
        envelope.push(measureLevels(audio.samples, frame, Math.min(frame + hop, range.last)).rmsDbfs);
    }
    const floor = Math.max(-65, envelope.reduce((peak, value) => Math.max(peak, value), dbFloor) - 40);
    const onsets = [];
    let lastOnset = -50;
    for (let index = 0; index < envelope.length; index++) {
        const baseline = envelope[Math.max(0, index - 5)];
        if (index - lastOnset < 25 || envelope[index] < floor || (index > 0 && envelope[index] < baseline + 8)) continue;

        const end = Math.min(index + 25, envelope.length);
        let peak = index;
        for (let next = index + 1; next < end; next++) {
            if (envelope[next] > envelope[peak]) peak = next;
        }
        let start = index;
        while (start > Math.max(0, index - 5) && envelope[start - 1] > Math.max(floor, envelope[peak] - 20)) start--;
        let attackEnd = start;
        while (attackEnd < peak && envelope[attackEnd] < envelope[peak] - 1) attackEnd++;

        lastOnset = peak;
        onsets.push({
            time: round((range.first + start * hop) / audio.rate, 6),
            attackMs: round((attackEnd - start) * hop * 1000 / audio.rate, 2),
            peakRmsDbfs: envelope[peak],
        });
    }
    return onsets;
}

function measureModulation(frames, hopSeconds) {
    return {
        pitch: modulationTrace(frames.map(frame => frame.pitch ? 1200 * Math.log2(frame.pitch.fundamentalHz / 440) : null), hopSeconds, "cents"),
        level: modulationTrace(frames.map(frame => frame.rmsDbfs > -65 ? frame.rmsDbfs : null), hopSeconds, "dB"),
        timbre: modulationTrace(frames.map(frame => frame.rmsDbfs > -65 ? frame.centroidHz : null), hopSeconds, "Hz"),
    };
}

function modulationTrace(values, hopSeconds, unit) {
    const sorted = values.filter(value => value !== null).sort((first, second) => first - second);
    if (sorted.length < 2) return {depth: null, unit, rateHz: null};

    const depth = sorted[Math.floor((sorted.length - 1) * 0.95)] - sorted[Math.floor((sorted.length - 1) * 0.05)];
    const mean = sorted.reduce((total, value) => total + value, 0) / sorted.length;
    const centered = values.map(value => value === null ? null : value - mean);
    const maxLag = Math.min(Math.floor(centered.length / 3), Math.ceil(2 / hopSeconds));
    const scores = [];
    for (let lag = 1; lag <= maxLag + 1; lag++) {
        scores[lag] = correlation(centered, centered, 0, lag, centered.length - lag) ?? 0;
    }
    let period = null;
    let valley = false;
    for (let lag = 2; lag <= maxLag; lag++) {
        if (scores[lag] < 0) valley = true;
        if (lag < 3 || !valley || scores[lag] < 0.6 || scores[lag] <= scores[lag - 1] || scores[lag] < scores[lag + 1]) continue;

        const offset = 0.5 * (scores[lag - 1] - scores[lag + 1]) / (scores[lag - 1] - 2 * scores[lag] + scores[lag + 1]);
        period = (lag + offset) * hopSeconds;
        break;
    }
    return {depth: round(depth, 2), unit, rateHz: period ? round(1 / period, 2) : null};
}

function correlation(left, right, first, second, count, stride = 1) {
    let cross = 0;
    let leftPower = 0;
    let rightPower = 0;
    let pairs = 0;
    for (let offset = 0; offset < count; offset += stride) {
        const a = left[first + offset];
        const b = right[second + offset];
        if (a === null || b === null) continue;

        cross += a * b;
        leftPower += a * a;
        rightPower += b * b;
        pairs++;
    }
    if (pairs < Math.min(8, Math.ceil(count / stride)) || !leftPower || !rightPower) return null;

    return Math.max(-1, Math.min(1, cross / Math.sqrt(leftPower * rightPower)));
}

function measureStereo(audio, range) {
    if (audio.samples.length < 2) return null;

    const [left, right] = audio.samples;
    const length = range.last - range.first;
    const zeroLag = correlation(left, right, range.first, range.first, length);
    let midPower = 0;
    let sidePower = 0;
    for (let frame = range.first; frame < range.last; frame++) {
        midPower += ((left[frame] + right[frame]) / 2) ** 2;
        sidePower += ((left[frame] - right[frame]) / 2) ** 2;
    }
    const stride = Math.max(1, Math.floor(length / 2048));
    const limit = Math.min(Math.round(audio.rate * 0.02), Math.floor(length / 4));
    let best = zeroLag === null ? 0 : Math.abs(zeroLag);
    let delayFrames = 0;
    if (zeroLag !== null && best < 0.999) {
        for (let lag = -limit; lag <= limit; lag++) {
            const first = range.first + Math.max(0, -lag);
            const second = range.first + Math.max(0, lag);
            const score = Math.abs(correlation(left, right, first, second, length - Math.abs(lag), stride) ?? 0);
            if (score > best + 0.001) {
                best = score;
                delayFrames = lag;
            }
        }
    }
    return {
        correlation: zeroLag === null ? null : round(zeroLag, 4),
        sideEnergyFraction: midPower + sidePower > 0 ? round(sidePower / (midPower + sidePower), 4) : null,
        delayMs: best >= 0.6 ? round(delayFrames * 1000 / audio.rate, 4) : null,
        delayConfidence: best >= 0.6 ? round(best, 3) : null,
    };
}

function measureImpulse(audio, range) {
    let peak = 0;
    let arrival = range.first;
    for (let frame = range.first; frame < range.last; frame++) {
        for (const channel of audio.samples) {
            const amplitude = Math.abs(channel[frame]);
            if (amplitude > peak) {
                peak = amplitude;
                arrival = frame;
            }
        }
    }
    if (!peak) return {peakTime: null, peakDbfs: dbFloor, t20Seconds: null, t30Seconds: null};

    const energy = new Float64Array(range.last - arrival);
    let total = 0;
    for (let frame = range.last - 1; frame >= arrival; frame--) {
        for (const channel of audio.samples) total += channel[frame] ** 2;
        energy[frame - arrival] = total;
    }
    const crossings = [-5, -25, -35].map(db => energy.findIndex(value => value <= total * 10 ** (db / 10)));
    const [start, end20, end30] = crossings;
    return {
        peakTime: round(arrival / audio.rate, 6),
        peakDbfs: toDb(peak ** 2),
        t20Seconds: end20 > start && start >= 0 ? round((end20 - start) * 3 / audio.rate, 4) : null,
        t30Seconds: end30 > start && start >= 0 ? round((end30 - start) * 2 / audio.rate, 4) : null,
    };
}

function measureLevels(channels, first, last) {
    let peak = 0;
    let power = 0;
    for (const channel of channels) {
        for (let frame = first; frame < last; frame++) {
            const sample = channel[frame];
            peak = Math.max(peak, Math.abs(sample));
            power += sample ** 2;
        }
    }

    return {peakDbfs: toDb(peak ** 2), rmsDbfs: toDb(power / ((last - first) * channels.length))};
}

function measureMel(spectrum, rate, range, hop, binHz, options, maxHz) {
    const melBands = options.melBands ?? 40;
    if (!Number.isInteger(melBands) || melBands < 8 || melBands > 96) throw new Error("melBands must be an integer from 8 to 96");

    const minHz = options.minHz ?? 0;
    if (!Number.isFinite(minHz) || minHz < 0 || minHz >= maxHz) throw new Error("minHz must be at least zero and below maxHz");

    const firstMel = Math.log1p(minHz / 700);
    const lastMel = Math.log1p(maxHz / 700);
    const spacing = (lastMel - firstMel) / (melBands + 1);
    const bandsHz = Array.from({length: melBands}, (_, i) => round(700 * Math.expm1(firstMel + (i + 1) * spacing), 1));
    const weights = [];
    for (let index = 0; index < melBands; index++) {
        const low = 700 * Math.expm1(firstMel + index * spacing);
        const center = 700 * Math.expm1(firstMel + (index + 1) * spacing);
        const high = 700 * Math.expm1(firstMel + (index + 2) * spacing);
        const entries = [];
        for (let bin = Math.ceil(low / binHz); bin <= Math.floor(high / binHz); bin++) {
            const hz = bin * binHz;
            const weight = Math.min((hz - low) / (center - low), (high - hz) / (high - center));
            if (weight > 0) entries.push({bin, weight});
        }
        weights.push(entries);
    }

    const times = [];
    const valuesDbfs = [];
    for (let first = range.first; first < range.last; first += hop) {
        const center = Math.floor((first + Math.min(first + hop, range.last)) / 2);
        const powers = spectrum(center);
        times.push(center / rate);
        valuesDbfs.push(weights.map(band => {
            let power = 0;
            for (const {bin, weight} of band) power += powers[bin] * weight;
            return toDb(power);
        }));
    }

    return {minHz, bandsHz, times, valuesDbfs};
}

function toDb(power) {
    return round(Math.max(dbFloor, 10 * Math.log10(power)), 2);
}

function round(value, digits) {
    const factor = 10 ** digits;
    return Math.round(value * factor) / factor;
}
