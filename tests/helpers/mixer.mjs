import {createMixer, encodeWav, maxSeconds, sampleRate} from "../../web/audio.mjs";

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
