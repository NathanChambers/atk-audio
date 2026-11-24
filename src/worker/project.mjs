import {createHash} from "node:crypto";
import {readFile, stat} from "node:fs/promises";
import {join} from "node:path";
import {decodeWav, maxSeconds, presetEvents, sampleRate, timelineDuration} from "../../web/audio.mjs";
import {encodeWav, mixAudio, render, synthesize} from "./audio.mjs";
import {createSpectrogram} from "./spectrogram.mjs";
import {analyzeAudio, melodyNotes} from "./analysis.mjs";

const sourceLimit = 64 * 1024 * 1024;

export function identifier(value) {
    if (typeof value !== "string" || !/^[a-z0-9][a-z0-9_-]{0,63}$/.test(value)) {
        throw new Error("Use a lowercase ID with letters, numbers, underscores or hyphens");
    }
    return value;
}

function number(value, min, max, name) {
    if (!Number.isFinite(value) || value < min || value > max) throw new Error(name + " must be between " + min + " and " + max);
    return value;
}

function find(items, id) {
    const item = items.find(entry => entry.id === id);
    if (!item) throw new Error("Unknown ID: " + id);
    return item;
}

async function readJson(path) {
    try {
        return JSON.parse(await readFile(path, "utf8"));
    } catch (error) {
        if (error.code === "ENOENT") return null;
        throw error;
    }
}

export function projectPaths(project) {
    const base = join(project, ".audio");
    return {
        base,
        assets: join(base, "assets"),
        sessions: join(base, "sessions"),
        sources: join(base, "sources"),
        lock: join(base, "write-lock"),
    };
}

export function sourcePath(paths, hash) {
    if (!/^[0-9a-f]{64}$/.test(hash)) throw new Error("Invalid source hash");
    return join(paths.sources, hash + ".wav");
}

function documentPath(directory, id) {
    return join(directory, identifier(id) + ".json");
}

export async function readDocument(paths, kind, id) {
    if (kind !== "assets" && kind !== "sessions") throw new Error("Unknown document kind");
    const document = await readJson(documentPath(paths[kind], id));
    if (!document) throw new Error("Unknown " + kind.slice(0, -1) + ": " + id);
    return document;
}

async function readSource(location) {
    if (typeof location !== "string" || !location) throw new Error("Provide a WAV path or URL");
    if (!/^https?:\/\//i.test(location)) {
        if ((await stat(location)).size > sourceLimit) throw new Error("WAV exceeds 64 MiB");
        return readFile(location);
    }

    const response = await fetch(location, {signal: AbortSignal.timeout(30000)});
    if (!response.ok || !response.body) throw new Error("Download failed: HTTP " + response.status);

    const chunks = [];
    let size = 0;
    for await (const chunk of response.body) {
        size += chunk.length;
        if (size > sourceLimit) throw new Error("WAV exceeds 64 MiB");
        chunks.push(chunk);
    }
    return Buffer.concat(chunks, size);
}

function synthesisSettings(command) {
    const wave = command.wave ?? "tone";
    if (wave === "formant") return formantSettings(command);
    if (!["tone", "noise", "additive"].includes(wave)) throw new Error("Wave must be tone, noise, additive or formant");
    const seconds = number(command.seconds ?? 1, 0.001, maxSeconds, "seconds");
    const frequency = number(command.frequency ?? 440, 1, 20000, "frequency");
    const settings = {
        wave, seconds, frequency,
        endFrequency: number(command.endFrequency ?? frequency, 1, 20000, "endFrequency"),
        attack: number(command.attack ?? Math.min(0.005, seconds), 0, seconds, "attack"),
        release: number(command.release ?? Math.min(0.005, seconds), 0, seconds, "release"),
        gain: number(command.gain ?? 0.7, 0, 1, "gain"),
        seed: Math.trunc(number(command.seed ?? 1, 0, 4294967295, "seed")),
        harmonics: Math.trunc(number(command.harmonics ?? 1, 1, 32, "harmonics")),
    };
    if (wave === "additive") {
        if (!Array.isArray(command.partials) || command.partials.length < 1 || command.partials.length > 32) {
            throw new Error("Additive synthesis requires 1 to 32 partials");
        }
        settings.partials = command.partials.map((partial, index) => {
            const name = "partials[" + index + "]";
            if (!partial || typeof partial !== "object" || Array.isArray(partial)) throw new Error(name + " must be an object");

            const frequency = number(partial.frequency, 1, 20000, name + ".frequency");
            return {
                frequency,
                endFrequency: number(partial.endFrequency ?? frequency, 1, 20000, name + ".endFrequency"),
                gain: number(partial.gain ?? 1, 0, 1, name + ".gain"),
                phaseDegrees: number(partial.phaseDegrees ?? 0, -360, 360, name + ".phaseDegrees"),
                decay: partial.decay === undefined ? null : number(partial.decay, 0.001, maxSeconds, name + ".decay"),
            };
        });
    }
    if (command.am !== undefined) {
        if (!command.am || typeof command.am !== "object" || Array.isArray(command.am)) throw new Error("am must be an object");

        settings.am = {
            frequency: number(command.am.frequency, 0.01, 1000, "am.frequency"),
            depth: number(command.am.depth, 0, 1, "am.depth"),
        };
    }
    return settings;
}

function formantSettings(command) {
    if (!Array.isArray(command.segments) || command.segments.length < 1 || command.segments.length > 1000) {
        throw new Error("Formant synthesis requires 1 to 1000 segments");
    }

    const segments = command.segments.map((segment, index) => {
        const name = "segments[" + index + "]";
        if (!segment || typeof segment !== "object" || Array.isArray(segment)) throw new Error(name + " must be an object");

        const formants = segment.formants ?? [];
        if (!Array.isArray(formants) || formants.length > 5) throw new Error(name + ".formants must contain up to five frequencies");

        const frequency = number(segment.frequency ?? 135, 40, 500, name + ".frequency");
        return {
            seconds: number(segment.seconds, 0.001, maxSeconds, name + ".seconds"),
            frequency,
            endFrequency: number(segment.endFrequency ?? frequency, 40, 500, name + ".endFrequency"),
            formants: formants.map(value => number(value, 80, 6000, name + ".formants")),
            noise: number(segment.noise ?? 0, 0, 1, name + ".noise"),
            gain: number(segment.gain ?? 1, 0, 1, name + ".gain"),
        };
    });
    const frames = segments.reduce((total, segment) => total + Math.round(segment.seconds * sampleRate), 0);
    number(frames / sampleRate, 0.001, maxSeconds, "total segment duration");
    return {
        wave: "formant", segments,
        transition: number(command.transition ?? 0, 0, 0.06, "transition"),
        gain: number(command.gain ?? 0.7, 0, 1, "gain"),
        seed: Math.trunc(number(command.seed ?? 1, 0, 4294967295, "seed")),
    };
}

export async function prepareSource(command) {
    const id = identifier(command.id);
    let bytes, provenance;
    if (command.command === "melody_import") {
        const wav = await readSource(command.location);
        const recording = decodeWav(wav);
        const notes = melodyNotes(recording);
        const last = notes.at(-1);
        const samples = new Float32Array(Math.round((last.start + last.duration) * sampleRate));
        for (const note of notes) {
            const frequency = 440 * 2 ** ((note.midi - 69) / 12);
            const tone = synthesize({
                wave: "tone", seconds: note.duration, frequency, endFrequency: frequency,
                attack: 0.008, release: 0.015, gain: 0.65, harmonics: 3,
            });
            const audio = decodeWav(tone);
            const start = Math.round(note.start * sampleRate);
            const available = Math.min(audio.samples[0].length, samples.length - start);
            samples.set(audio.samples[0].subarray(0, available), start);
        }
        bytes = encodeWav([samples]);
        provenance = {location: command.location, notes};
    } else if (command.command === "source_generate") {
        const settings = synthesisSettings(command);
        bytes = synthesize(settings);
        provenance = {settings};
    } else {
        bytes = await readSource(command.location);
        provenance = {location: command.location, license: command.license ?? "", attribution: command.attribution ?? ""};
    }

    const audio = decodeWav(bytes);
    const hash = createHash("sha256").update(bytes).digest("hex");
    const source = {
        id,
        hash,
        duration: audio.duration,
        rate: audio.rate,
        channels: audio.samples.length,
        prompt: command.prompt ?? "",
        provenance,
    };
    return {source, bytes};
}

async function loadSources(paths, asset) {
    const used = new Set(asset.tracks.flatMap(track => track.clips.map(clip => clip.source)));
    const entries = await Promise.all(asset.sources.filter(source => used.has(source.id)).map(async source => {
        const bytes = await readFile(sourcePath(paths, source.hash));
        return [source.id, decodeWav(bytes)];
    }));
    return Object.fromEntries(entries);
}

async function audioForAnalysis(paths, sessionId, command) {
    const scope = command.scope ?? "mix";
    if (!["wav", "source", "clip", "track", "mix"].includes(scope)) throw new Error("Expected wav, source, clip, track or mix scope");

    if (scope === "wav") {
        if (command.command === "audio_analyze" && command.preset !== undefined) throw new Error("Presets apply to rendered audio");
        const bytes = await readSource(command.location);
        return {scope, audio: decodeWav(bytes)};
    }

    const {asset} = command.asset === undefined
        ? await readDocument(paths, "sessions", sessionId)
        : await readDocument(paths, "assets", command.asset);
    const preset = command.preset === undefined ? null : find(asset.presets ?? [], command.preset);
    if (preset && scope === "source") throw new Error("Presets apply to rendered audio");
    if (preset && command.events !== undefined) throw new Error("Choose a preset or explicit events");

    if (scope === "source") {
        const source = find(asset.sources, command.id);
        const bytes = await readFile(sourcePath(paths, source.hash));
        return {scope, id: source.id, audio: decodeWav(bytes)};
    }

    let isolated = asset;
    let trigger = null;
    if (scope === "track") {
        const track = find(asset.tracks, command.id);
        isolated = {...asset, tracks: [{...track, mute: false, solo: false}]};
    }
    if (scope === "clip") {
        const track = asset.tracks.find(track => track.clips.some(clip => clip.id === command.id));
        if (!track) throw new Error("Unknown clip: " + command.id);

        const clip = find(track.clips, command.id);
        trigger = clip.trigger;
        isolated = {...asset, tracks: [{...track, mute: false, solo: false, clips: [{...clip, start: 0}]}]};
    }

    if (command.events !== undefined && !Array.isArray(command.events)) throw new Error("events must be an array");
    const events = preset ? presetEvents(preset) : (command.events ?? []).map(event => {
        if (!event || typeof event !== "object") throw new Error("Expected a control event");
        const control = find(asset.controls, event.control);
        const time = number(event.time, 0, maxSeconds, "event time");
        const value = control.type === "slider" ? number(event.value, control.min, control.max, "control value") : undefined;
        return {control: control.id, value, time};
    });
    if (trigger && !events.some(event => event.control === trigger)) events.unshift({control: trigger, time: 0});

    let duration = timelineDuration(isolated);
    for (const track of isolated.tracks) {
        for (const clip of track.clips) {
            if (!clip.trigger) continue;
            for (const event of events) {
                if (event.control === clip.trigger) duration = Math.max(duration, event.time + clip.duration);
            }
        }
    }
    if (command.sustain === true && Number.isFinite(command.end)) duration = Math.max(duration, command.end);
    duration = preset?.seconds ?? command.seconds ?? Math.min(maxSeconds, duration);
    duration = number(duration, 1 / sampleRate, maxSeconds, "seconds");
    const roundedDuration = Math.ceil(duration * sampleRate) / sampleRate;
    const sources = await loadSources(paths, isolated);
    const mixed = mixAudio(isolated, sources, roundedDuration, events, preset?.sustain ?? command.sustain === true);
    return {scope, id: command.id ?? asset.id, audio: mixed};
}

export async function runCommand(paths, sessionId, command) {
    if (!command || typeof command !== "object" || Array.isArray(command)) throw new Error("Expected a JSON command");
    if (["spectrogram", "audio_analyze"].includes(command.command)) {
        const {audio, ...metadata} = await audioForAnalysis(paths, sessionId, command);
        if (command.command === "spectrogram") return {...metadata, ...createSpectrogram(audio, command)};

        const preset = command.preset === undefined ? {} : {preset: command.preset};
        return {...metadata, ...preset, ...analyzeAudio(audio, command)};
    }
    if (command.command === "session_render") {
        const {asset} = await readDocument(paths, "sessions", sessionId);
        const preset = command.preset === undefined ? null : find(asset.presets ?? [], command.preset);
        const sources = await loadSources(paths, asset);
        const duration = preset?.seconds ?? command.seconds ?? timelineDuration(asset);
        const events = preset ? presetEvents(preset) : [];
        return render(asset, sources, duration, events, preset?.sustain ?? command.sustain === true);
    }

    throw new Error("Unknown processing command");
}
