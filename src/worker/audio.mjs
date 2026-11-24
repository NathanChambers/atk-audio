import {createMixer, encodeWav, maxSeconds, sampleRate} from "../../web/audio.mjs";

export {encodeWav};

export function synthesize(settings) {
    if (settings.wave === "formant") return synthesizeFormants(settings);

    const {wave, seconds, frequency, endFrequency, attack, release, gain, seed, harmonics} = settings;
    const frames = Math.round(seconds * sampleRate);
    const samples = new Float32Array(frames);
    let random = seed;
    let phase = 0;
    let filtered = 0;
    let harmonicGain = 0;
    for (let harmonic = 1; harmonic <= harmonics; harmonic++) harmonicGain += 1 / (harmonic * harmonic);

    const partials = (settings.partials ?? []).map(partial => ({
        phase: partial.phaseDegrees * Math.PI / 180,
        step: 2 * Math.PI * partial.frequency / sampleRate,
        sweep: 2 * Math.PI * (partial.endFrequency - partial.frequency) / (sampleRate ** 2 * seconds),
        amplitude: partial.gain,
        damping: partial.decay === null ? 1 : Math.exp(-1 / (partial.decay * sampleRate)),
    }));
    const partialGain = Math.max(1, partials.reduce((total, partial) => total + partial.amplitude, 0));
    const modulation = settings.am;

    for (let frame = 0; frame < frames; frame++) {
        const time = frame / sampleRate;
        const pitch = frequency + (endFrequency - frequency) * time / seconds;
        let value = 0;
        if (wave === "noise") {
            random = (Math.imul(random, 1664525) + 1013904223) >>> 0;
            const alpha = 1 - Math.exp(-2 * Math.PI * pitch / sampleRate);
            filtered += alpha * (random / 2147483648 - 1 - filtered);
            value = filtered;
        } else if (wave === "additive") {
            for (const partial of partials) {
                value += partial.amplitude * Math.sin(partial.phase);
                partial.phase = (partial.phase + partial.step) % (2 * Math.PI);
                partial.step += partial.sweep;
                partial.amplitude *= partial.damping;
            }
            value /= partialGain;
        } else {
            for (let harmonic = 1; harmonic <= harmonics && pitch * harmonic < sampleRate / 2; harmonic++) {
                value += Math.sin(phase * harmonic) / (harmonic * harmonic);
            }
            value /= harmonicGain;
            phase = (phase + 2 * Math.PI * pitch / sampleRate) % (2 * Math.PI);
        }
        const fadeIn = attack ? Math.min(1, time / attack) : 1;
        const fadeOut = release ? Math.min(1, (frames - 1 - frame) / (sampleRate * release)) : 1;
        if (modulation) value *= 1 - modulation.depth * (1 - Math.cos(2 * Math.PI * modulation.frequency * time)) / 2;
        samples[frame] = value * gain * Math.min(fadeIn, fadeOut);
    }

    return encodeWav([samples]);
}

function synthesizeFormants({segments, gain, seed, transition = 0}) {
    const lengths = segments.map(segment => Math.round(segment.seconds * sampleRate));
    const samples = new Float32Array(lengths.reduce((total, length) => total + length, 0));
    let offset = 0;
    let phase = 0;
    let random = seed;
    let previousNoise = 0;
    let peak = 0;
    const transitionFrames = Math.round(transition * sampleRate);

    for (const [index, segment] of segments.entries()) {
        const {frequency, endFrequency, formants, noise} = segment;
        const previous = transitionFrames ? segments[index - 1] : undefined;
        const length = lengths[index];
        const glide = Math.min(transitionFrames, Math.floor(length / 2));
        const harmonics = formants.length || previous?.formants.length ? 48 : 0;
        const ramp = Math.round(sampleRate * 0.005);
        const levels = new Float64Array(harmonics);
        for (let frame = 0; frame < length; frame++) {
            const blend = previous && glide ? Math.min(1, frame / glide) : 1;
            const targetPitch = frequency + (endFrequency - frequency) * frame / length;
            const pitch = blend < 1 ? previous.endFrequency + (targetPitch - previous.endFrequency) * blend : targetPitch;
            if (frame % 64 === 0) {
                for (let harmonic = 1; harmonic <= harmonics; harmonic++) {
                    const harmonicFrequency = harmonic * pitch;
                    let level = formantLevel(formants, harmonicFrequency) * blend;
                    if (blend < 1) {
                        level += formantLevel(previous.formants, harmonicFrequency) * (1 - blend);
                    }
                    levels[harmonic - 1] = level / Math.sqrt(harmonic);
                }
            }

            phase = (phase + 2 * Math.PI * pitch / sampleRate) % (2 * Math.PI);
            let value = 0;
            for (let harmonic = 1; harmonic <= harmonics; harmonic++) {
                value += levels[harmonic - 1] * Math.sin(harmonic * phase);
            }
            const noiseLevel = blend < 1 ? previous.noise + (noise - previous.noise) * blend : noise;
            if (noiseLevel > 0) {
                random = (Math.imul(random, 1664525) + 1013904223) >>> 0;
                const current = random / 2147483648 - 1;
                value += noiseLevel * (current - 0.7 * previousNoise);
                previousNoise = current;
            }
            const attack = previous ? 1 : frame / ramp;
            const release = transitionFrames && index < segments.length - 1 ? 1 : (length - 1 - frame) / ramp;
            const envelope = Math.min(1, attack, release);
            const segmentGain = blend < 1 ? previous.gain + (segment.gain - previous.gain) * blend : segment.gain;
            const sample = value * segmentGain * envelope;
            samples[offset + frame] = sample;
            peak = Math.max(peak, Math.abs(sample));
        }
        offset += length;
    }

    if (peak > 0) {
        const scale = gain / peak;
        for (let frame = 0; frame < samples.length; frame++) samples[frame] *= scale;
    }
    return encodeWav([samples]);
}

function formantLevel(formants, frequency) {
    let level = 0;
    for (const [band, resonance] of formants.entries()) {
        const distance = (frequency - resonance) / (90 + band * 60);
        level += 1 / ((1 + band * 0.6) * (1 + distance * distance));
    }
    return level;
}

export function mixAudio(asset, sources, duration, events = [], sustain = false) {
    if (!Number.isFinite(duration) || duration <= 0 || duration > maxSeconds) throw new Error("Invalid render duration");

    const frames = Math.max(1, Math.round(duration * sampleRate));
    const samples = [new Float32Array(frames), new Float32Array(frames)];
    const timedEvents = events.map(event => ({...event, frame: Math.round(event.time * sampleRate)}));
    timedEvents.sort((first, second) => first.frame - second.frame);
    const mixer = createMixer(asset, sources);
    mixer.mix(samples[0], samples[1], timedEvents, sustain);
    return {samples, rate: sampleRate, duration: frames / sampleRate};
}

export function render(asset, sources, duration, events = [], sustain = false) {
    const mixed = mixAudio(asset, sources, duration, events, sustain);
    let peak = 0;
    let clippedSamples = 0;
    for (const channel of mixed.samples) {
        for (const sample of channel) {
            const level = Math.abs(sample);
            if (level > 1) clippedSamples++;
            peak = Math.max(peak, level);
        }
    }

    return {bytes: encodeWav(mixed.samples), peak, clippedSamples, duration: mixed.duration};
}
