export const sampleRate = 48000;
export const maxSeconds = 120;

export function encodeWav(samples, rate = sampleRate) {
    const frames = samples[0].length;
    const channels = samples.length;
    const size = frames * channels * 2;
    const bytes = new Uint8Array(44 + size);
    const view = new DataView(bytes.buffer);
    const tag = (offset, value) => bytes.set(new TextEncoder().encode(value), offset);
    tag(0, "RIFF");
    view.setUint32(4, size + 36, true);
    tag(8, "WAVE");
    tag(12, "fmt ");
    view.setUint32(16, 16, true);
    view.setUint16(20, 1, true);
    view.setUint16(22, channels, true);
    view.setUint32(24, rate, true);
    view.setUint32(28, rate * channels * 2, true);
    view.setUint16(32, channels * 2, true);
    view.setUint16(34, 16, true);
    tag(36, "data");
    view.setUint32(40, size, true);
    for (let frame = 0; frame < frames; frame++) {
        for (let channel = 0; channel < channels; channel++) {
            const value = Math.max(-1, Math.min(1, samples[channel][frame]));
            view.setInt16(44 + (frame * channels + channel) * 2, Math.round(value * (value < 0 ? 32768 : 32767)), true);
        }
    }

    return bytes;
}

export function decodeWav(bytes) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const tag = offset => String.fromCharCode(...bytes.subarray(offset, offset + 4));
    if (bytes.length < 12 || tag(0) !== "RIFF" || tag(8) !== "WAVE") throw new Error("Expected RIFF WAV");

    let format;
    let data;
    for (let offset = 12; offset + 8 <= bytes.length;) {
        const size = view.getUint32(offset + 4, true);
        const start = offset + 8;
        if (start + size > bytes.length) throw new Error("Truncated WAV");
        if (tag(offset) === "fmt " && size >= 16) {
            format = {
                encoding: view.getUint16(start, true),
                channels: view.getUint16(start + 2, true),
                rate: view.getUint32(start + 4, true),
                alignment: view.getUint16(start + 12, true),
                bits: view.getUint16(start + 14, true),
            };
        }
        if (tag(offset) === "data") data = {start, size};
        offset = start + size + size % 2;
    }

    const pcm = format?.encoding === 1 && [8, 16, 24, 32].includes(format.bits);
    const float = format?.encoding === 3 && format.bits === 32;
    if ((!pcm && !float) || !data) throw new Error("Expected PCM or 32-bit float WAV");

    const {channels, rate, bits, alignment} = format;
    const width = bits / 8;
    if (![1, 2].includes(channels) || rate < 8000 || rate > 192000 || alignment !== channels * width) {
        throw new Error("Expected mono or stereo WAV at 8–192 kHz");
    }

    const frames = Math.floor(data.size / alignment);
    if (frames < 1 || frames / rate > maxSeconds) throw new Error("WAV must contain up to 120 seconds of audio");
    const samples = Array.from({length: channels}, () => new Float32Array(frames));
    for (let frame = 0; frame < frames; frame++) {
        for (let channel = 0; channel < channels; channel++) {
            const offset = data.start + frame * alignment + channel * width;
            let value;
            if (float) value = view.getFloat32(offset, true);
            else if (bits === 8) value = (view.getUint8(offset) - 128) / 128;
            else if (bits === 16) value = view.getInt16(offset, true) / 32768;
            else if (bits === 24) value = (view.getUint16(offset, true) + view.getInt8(offset + 2) * 65536) / 8388608;
            else value = view.getInt32(offset, true) / 2147483648;
            if (!Number.isFinite(value)) throw new Error("WAV contains invalid samples");
            samples[channel][frame] = Math.max(-1, Math.min(1, value));
        }
    }

    return {rate, samples, duration: frames / rate};
}

export function timelineDuration(asset) {
    return Math.max(0, ...asset.tracks.flatMap(track => track.clips.filter(clip => !clip.trigger).map(clip => clip.start + clip.duration)));
}

export function presetValue(points, time) {
    const next = points.findIndex(point => point.time > time);
    if (next === 0) return points[0].value;
    if (next === -1) return points.at(-1).value;

    const first = points[next - 1];
    const second = points[next];
    const fraction = (time - first.time) / (second.time - first.time);
    return first.value + fraction * (second.value - first.value);
}

export function presetEvents(preset) {
    const events = [...preset.triggers];
    for (const lane of preset.lanes) {
        const {control, points} = lane;
        events.push({control, time: 0, value: points[0].value});
        for (let index = 1; index < points.length; index++) {
            const first = points[index - 1];
            const second = points[index];
            const span = second.time - first.time;
            const steps = Math.ceil(span / 0.005);
            for (let step = 1; step < steps; step++) {
                const fraction = step / steps;
                events.push({control, time: first.time + fraction * span, value: first.value + fraction * (second.value - first.value)});
            }
            events.push({control, ...second});
        }
    }
    return events.sort((first, second) => first.time - second.time);
}

export function createMixer(asset, sources, rate = sampleRate, trackMonitor = null) {
    let frame = 0;
    const values = Object.fromEntries(asset.controls.filter(control => control.type === "slider").map(control => [control.id, control.value]));
    const voices = [];
    const trackBuses = [];
    const monitoring = trackMonitor !== null;
    const monitorStep = 1 / Math.max(1, Math.round(rate * 0.005));
    const solo = asset.tracks.some(track => track.solo && !track.mute);
    for (const track of asset.tracks) {
        if (!monitoring && (track.mute || (solo && !track.solo))) continue;
        const filter = createTrackFilter(track, rate);
        const bus = filter || monitoring ? {track, filter, left: 0, right: 0, gain: 1, target: 1} : null;
        if (bus) trackBuses.push(bus);
        for (const clip of track.clips) {
            const source = sources[clip.source];
            const trim = clip.trim * source.rate;
            const span = Math.min(source.samples[0].length - trim, clip.length * source.rate);
            const pan = Math.max(-1, Math.min(1, track.pan + clip.pan));
            const gain = track.gain * clip.gain;
            voices.push({
                clip, source, trim, span, bus,
                start: clip.trigger ? Infinity : Math.round(clip.start * rate),
                position: 0,
                left: gain * Math.min(1, 1 - pan),
                right: gain * Math.min(1, 1 + pan),
                mappings: asset.mappings.filter(mapping => mapping.clip === clip.id),
            });
        }
    }

    if (monitoring) setTrackMonitor(trackMonitor, true);

    function setTrackMonitor(settings, immediate = false) {
        if (!monitoring) return;

        const states = trackBuses.map(bus => {
            const {track} = bus;
            const override = settings[track.id];
            return {bus, mute: override?.mute ?? track.mute, solo: override?.solo ?? track.solo};
        });
        const hasSolo = states.some(state => state.solo && !state.mute);
        for (const {bus, mute, solo} of states) {
            bus.target = mute || (hasSolo && !solo) ? 0 : 1;
            if (immediate) bus.gain = bus.target;
        }
    }

    function apply(event) {
        if (Object.hasOwn(values, event.control)) values[event.control] = event.value;
        for (const voice of voices) {
            if (voice.clip.trigger !== event.control) continue;
            voice.start = frame;
            voice.position = 0;
        }
    }

    function mix(left, right, events = [], sustain = false) {
        let nextEvent = 0;
        for (let offset = 0; offset < left.length; offset++, frame++) {
            while (nextEvent < events.length && events[nextEvent].frame <= frame) {
                apply(events[nextEvent]);
                nextEvent++;
            }

            let totalLeft = 0;
            let totalRight = 0;
            for (const voice of voices) {
                const {clip, source, span, trim} = voice;
                const elapsed = (frame - voice.start) / rate;
                const duration = sustain && clip.loop && !clip.trigger ? Infinity : clip.duration;
                if (elapsed < 0 || elapsed >= duration || span <= 0) continue;
                if (!clip.loop && voice.position >= span) continue;

                let gain = 1;
                let pitch = 1;
                for (const mapping of voice.mappings) {
                    const controlValue = values[mapping.control];
                    const position = (controlValue - mapping.inputMin) / (mapping.inputMax - mapping.inputMin);
                    const fraction = Math.max(0, Math.min(1, position));
                    const value = mapping.curve === "equal-power"
                        ? Math.sqrt(mapping.min ** 2 * (1 - fraction) + mapping.max ** 2 * fraction)
                        : mapping.min + (mapping.max - mapping.min) * fraction;
                    if (mapping.target === "gain") gain *= value;
                    else pitch *= value;
                }

                const position = trim + (voice.position % span);
                const index = Math.floor(position);
                const next = index + 1 < trim + span ? index + 1 : Math.floor(clip.loop ? trim : position);
                const fraction = position - index;
                const first = source.samples[0];
                const second = source.samples[1] ?? first;
                const fadeIn = clip.fadeIn ? Math.min(1, elapsed / clip.fadeIn) : 1;
                const fadeOut = clip.fadeOut ? Math.min(1, (duration - elapsed) / clip.fadeOut) : 1;
                const scale = gain * Math.min(fadeIn, fadeOut);
                const sampleLeft = (first[index] + ((first[next] ?? first[index]) - first[index]) * fraction) * scale * voice.left;
                const sampleRight = (second[index] + ((second[next] ?? second[index]) - second[index]) * fraction) * scale * voice.right;
                if (voice.bus) {
                    voice.bus.left += sampleLeft;
                    voice.bus.right += sampleRight;
                } else {
                    totalLeft += sampleLeft;
                    totalRight += sampleRight;
                }
                voice.position += pitch * source.rate / rate;
            }
            for (const bus of trackBuses) {
                if (bus.gain !== bus.target) {
                    const difference = bus.target - bus.gain;
                    bus.gain += Math.max(-monitorStep, Math.min(monitorStep, difference));
                }
                const sampleLeft = bus.filter ? bus.filter(bus.left, 0) : bus.left;
                const sampleRight = bus.filter ? bus.filter(bus.right, 1) : bus.right;
                totalLeft += sampleLeft * bus.gain;
                totalRight += sampleRight * bus.gain;
                bus.left = 0;
                bus.right = 0;
            }
            left[offset] = totalLeft;
            right[offset] = totalRight;
        }
    }

    return {mix, setTrackMonitor};
}

function createTrackFilter(track, rate) {
    const highPass = track.highPassHz ? createPassFilter(track.highPassHz, rate, true) : null;
    const lowPass = track.lowPassHz ? createPassFilter(track.lowPassHz, rate, false) : null;
    if (highPass && lowPass) return (input, channel) => lowPass(highPass(input, channel), channel);
    return highPass ?? lowPass;
}

function createPassFilter(frequency, rate, highPass) {
    const angle = 2 * Math.PI * Math.min(frequency, rate * 0.45) / rate;
    const cosine = Math.cos(angle);
    const alpha = Math.sin(angle) / Math.SQRT2;
    const divisor = 1 + alpha;
    const b0 = (highPass ? 1 + cosine : 1 - cosine) / (2 * divisor);
    const b1 = (highPass ? -2 : 2) * b0;
    const a1 = -2 * cosine / divisor;
    const a2 = (1 - alpha) / divisor;
    const first = [0, 0];
    const second = [0, 0];
    return (input, channel) => {
        const output = b0 * input + first[channel];
        first[channel] = b1 * input - a1 * output + second[channel];
        second[channel] = b0 * input - a2 * output;
        return output;
    };
}
