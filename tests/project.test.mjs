import assert from "node:assert/strict";
import {execFile} from "node:child_process";
import {createServer} from "node:http";
import {mkdtemp, readFile, rm, writeFile} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join, resolve} from "node:path";
import {test} from "node:test";
import {fileURLToPath} from "node:url";
import {promisify} from "node:util";
import {createMixer, decodeWav, presetEvents, presetValue} from "../web/audio.mjs";
import {mixAudio, render} from "../src/worker/audio.mjs";
import {analyzeAudio} from "../src/worker/analysis.mjs";
import {playbackCommands} from "./helpers/playback.mjs";

const execute = promisify(execFile);
const binaryName = process.platform === "win32" ? "atk-audio-cli.exe" : "atk-audio-cli";
const cli = process.env.ATK_AUDIO_TEST_CLI
    ? resolve(process.env.ATK_AUDIO_TEST_CLI)
    : fileURLToPath(new URL(`../target/debug/${binaryName}`, import.meta.url));

test("audio analysis measures pitch, onsets, modulation, stereo delay and impulse decay", () => {
    const rate = 8000;
    const tone = Float32Array.from({length: rate * 4}, (_, frame) => {
        const time = frame / rate;
        if (time < 0.1) return 0;

        const attack = Math.min(1, (time - 0.1) / 0.025);
        return attack * (0.5 + 0.2 * Math.sin(2 * Math.PI * 4 * time)) * Math.sin(2 * Math.PI * 200 * time);
    });
    const features = analyzeAudio({rate, duration: tone.length / rate, samples: [tone]}, {hopMs: 20});
    assert.equal(features.stereo, null);
    assert.equal(features.frames[0].pitch, null);
    assert.ok(Math.abs(features.frames[30].pitch.fundamentalHz - 200) < 2);
    assert.ok(Math.abs(features.onsets[0].time - 0.1) < 0.015);
    assert.ok(features.onsets[0].attackMs > 0);
    assert.ok(Math.abs(features.modulation.level.rateHz - 4) < 0.3);

    let seed = 3;
    const left = Float32Array.from({length: 4000}, () => {
        seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
        return seed / 2147483648 - 1;
    });
    const right = new Float32Array(left.length);
    right.set(left.subarray(0, left.length - 24), 24);
    const stereo = analyzeAudio({rate, duration: left.length / rate, samples: [left, right]}, {}).stereo;
    assert.equal(stereo.delayMs, 3);
    assert.ok(Math.abs(stereo.correlation) < 0.1);
    assert.ok(Math.abs(stereo.sideEnergyFraction - 0.5) < 0.05);

    const response = Float32Array.from({length: rate * 2}, (_, frame) => {
        if (frame < 80) return 0;
        return 10 ** (-3 * (frame - 80) / rate);
    });
    const impulse = analyzeAudio({rate, duration: 2, samples: [response]}, {format: "impulse"});
    assert.equal(impulse.peakTime, 0.01);
    assert.ok(Math.abs(impulse.t20Seconds - 1) < 0.01);
    assert.ok(Math.abs(impulse.t30Seconds - 1) < 0.01);
});

test("session drafts, WAV imports, playback controls and publish conflicts", async () => {
    const scratch = await mkdtemp(join(tmpdir(), "audio-workbench-"));
    const run = async (session, input, output) => {
        const args = ["--session", session, "--command", JSON.stringify(input)];
        if (output) args.push("--output", output);

        const {stdout} = await execute(cli, args, {cwd: scratch});
        return stdout.trim().split("\n").map(line => JSON.parse(line).result);
    };
    const command = async (session, name, fields = {}, output) => {
        const [result] = await run(session, {command: name, ...fields}, output);
        return result;
    };
    try {
        await run("playback-test", playbackCommands);
        const draft = await command("playback-test", "session_inspect");
        const sources = {};
        for (const source of draft.asset.sources) {
            const bytes = await readFile(join(scratch, ".audio", "sources", `${source.hash}.wav`));
            sources[source.id] = decodeWav(bytes);
        }

        const baseline = render(draft.asset, sources, 0.6);
        const blended = render(draft.asset, sources, 0.6, [{control: "blend", value: 6800, time: 0.1}]);
        const accented = render(draft.asset, sources, 0.6, [{control: "accent", time: 0.2}]);
        assert.ok(baseline.peak > 0.1);
        assert.equal(baseline.clippedSamples, 0);
        assert.notDeepEqual(baseline.bytes, blended.bytes);
        assert.notDeepEqual(baseline.bytes, accented.bytes);
        const audio = decodeWav(accented.bytes);
        assert.equal(audio.duration, 0.6);
        assert.equal(audio.samples.length, 2);

        await command("playback-test", "preset_set", {
            id: "sweep",
            seconds: 0.6,
            lanes: [{control: "blend", points: [{time: 0, value: 800}, {time: 0.4, value: 6800}]}],
            triggers: [{control: "accent", time: 0.2}],
        });
        const saved = await command("playback-test", "session_inspect");
        const [preset] = saved.asset.presets;
        assert.equal(presetValue(preset.lanes[0].points, 0.2), 3800);
        assert.equal(presetValue(preset.lanes[0].points, 0.59), 6800);
        await command("playback-test", "session_render", {preset: "sweep"}, "sweep.wav");
        const rendered = await readFile(join(scratch, "sweep.wav"));
        const expected = render(draft.asset, sources, 0.6, presetEvents(preset));
        assert.ok(rendered.equals(expected.bytes), "CLI export differs from the expected preset render");
        const waveform = await command("playback-test", "audio_analyze", {format: "waveform", preset: "sweep"}, "waveform.png");
        const png = await readFile(join(scratch, "waveform.png"));
        assert.equal(png.readUInt32BE(16), 900);
        assert.equal(waveform.end, 0.6);
        const pcm = await command("playback-test", "audio_analyze", {
            format: "pcm", scope: "source", id: "low-tone", start: 0.01, end: 0.02,
        });
        assert.equal(pcm.firstFrame, 480);
        assert.deepEqual(pcm.samples[0], Array.from(sources["low-tone"].samples[0].slice(480, 960)));
        await assert.rejects(command("playback-test", "audio_analyze", {format: "pcm", scope: "source", id: "low-tone"}), /8192 frames/);
        await assert.rejects(command("playback-test", "preset_set", {
            ...preset,
            lanes: [{control: "blend", points: [{time: 0.1, value: 800}, {time: 0.1, value: 900}]}],
        }), /increase/);

        const filename = join(scratch, "import.wav");
        await writeFile(filename, baseline.bytes);
        const local = await command("playback-test", "source_import", {id: "local", location: filename});
        const server = createServer((request, response) => response.end(baseline.bytes));
        await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
        try {
            const location = "http://127.0.0.1:" + server.address().port + "/import.wav";
            const remote = await command("playback-test", "source_import", {id: "remote", location, license: "Test"});
            assert.equal(remote.hash, local.hash);
            assert.equal(remote.provenance.license, "Test");
        } finally {
            await new Promise(resolve => server.close(resolve));
        }

        await command("playback-test", "track_add", {id: "import"});
        await command("playback-test", "clip_add", {id: "texture", track: "import", source: "local", start: 0.1});
        await command("playback-test", "clip_split", {id: "texture", newId: "tail", time: 0.35});
        await command("playback-test", "clip_edit", {id: "tail", track: "high", gain: 0.5});
        const edited = await command("playback-test", "session_inspect");
        const tail = edited.asset.tracks.find(track => track.id === "high").clips.find(clip => clip.id === "tail");
        assert.ok(Math.abs(tail.trim - 0.25) < 1e-8);

        const published = await command("playback-test", "session_publish");
        assert.equal(published.revision, 1);
        assert.deepEqual(published.asset.presets, [preset]);
        await command("agent-a", "session_start", {asset: "playback"});
        await command("agent-b", "session_start", {asset: "playback"});
        await command("agent-a", "asset_name", {name: "Variant A"});
        await command("agent-b", "asset_name", {name: "Variant B"});
        const next = await command("agent-a", "session_publish");
        assert.equal(next.revision, 2);
        await assert.rejects(command("agent-b", "session_publish"), /Publish conflict/);
        assert.equal((await command("agent-a", "session_inspect")).asset.name, "Variant A");
        assert.equal((await command("agent-b", "session_inspect")).asset.name, "Variant B");
    } finally {
        await rm(scratch, {recursive: true, force: true});
    }
});

test("analysis features and mel bands preserve spectral energy in stereo", () => {
    const rate = 48000;
    const samples = Float32Array.from({length: rate}, (_, frame) => 0.5 * Math.sin(2 * Math.PI * 750 * frame / rate));
    const inverted = samples.map(sample => -sample);
    const tone = {rate, duration: 1, samples: [samples, inverted]};
    const analysis = analyzeAudio(tone, {start: 0.2, end: 0.4, hopMs: 100});
    assert.equal(analysis.frames.length, 2);
    const frame = analysis.frames[0];
    assert.equal(frame.centroidHz, 750);
    assert.equal(frame.peakDbfs, -6.02);
    assert.equal(frame.rmsDbfs, -9.03);
    assert.equal(frame.bandDbfs.indexOf(Math.max(...frame.bandDbfs)), 3);
    assert.equal(frame.flux, null);
    assert.equal(analysis.frames[1].flux, 0);

    const mel = analyzeAudio(tone, {format: "mel", start: 0.2, end: 0.4, hopMs: 100, melBands: 32});
    const maximum = Math.max(...mel.valuesDbfs[0]);
    const peakHz = mel.bandsHz[mel.valuesDbfs[0].indexOf(maximum)];
    assert.ok(Math.abs(peakHz - 750) < 100);
});

test("a crossfade clamps its range and multiplies with level gain", () => {
    const clip = {id: "bank", source: "tone", start: 0, trim: 0, length: 1, duration: 1, gain: 1, pan: 0};
    const asset = {
        tracks: [{gain: 1, pan: 0, clips: [clip]}],
        controls: [{id: "blend", type: "slider", value: 1000}, {id: "level", type: "slider", value: 50}],
        mappings: [
            {clip: "bank", target: "gain", control: "level", inputMin: 0, inputMax: 100, min: 0, max: 1},
            {clip: "bank", target: "gain", control: "blend", inputMin: 3000, inputMax: 5000, min: 0, max: 1, curve: "equal-power"},
        ],
    };
    const sources = {tone: {rate: 48000, samples: [new Float32Array(48000).fill(1)]}};
    const events = [{time: 0.1, control: "blend", value: 4000}, {time: 0.2, control: "blend", value: 6000}];
    const {samples} = mixAudio(asset, sources, 0.3, events);
    assert.equal(samples[0][0], 0);
    assert.ok(Math.abs(samples[0][4800] - 0.5 * Math.SQRT1_2) < 1e-6);
    assert.equal(samples[0][9600], 0.5);
});

test("live mute and solo preserve the track playhead and saved settings", () => {
    const clip = {start: 0, trim: 0, length: 1, duration: 1, gain: 1, pan: 0};
    const asset = {
        controls: [],
        mappings: [],
        tracks: [
            {id: "body", gain: 1, pan: 0, clips: [{...clip, source: "ramp"}]},
            {id: "air", gain: 1, pan: 0, mute: true, clips: [{...clip, source: "constant"}]},
        ],
    };
    const ramp = Float32Array.from({length: 1000}, (_, frame) => frame / 1000);
    const sources = {
        ramp: {rate: 1000, samples: [ramp]},
        constant: {rate: 1000, samples: [new Float32Array(1000).fill(0.5)]},
    };
    const mixer = createMixer(asset, sources, 1000, {});
    const left = new Float32Array(100);
    const right = new Float32Array(100);
    mixer.mix(left, right);
    assert.equal(left[99], ramp[99]);

    mixer.setTrackMonitor({air: {mute: false, solo: true}});
    mixer.mix(left, right);
    assert.equal(left[99], 0.5);

    mixer.setTrackMonitor({});
    mixer.mix(left, right);
    assert.equal(left[99], ramp[299]);
    assert.equal(asset.tracks[1].mute, true);
});

test("track high-pass removes low frequencies while passing upper harmonics", () => {
    const rate = 48000;
    const frames = Float32Array.from({length: rate}, (_, i) =>
        0.25 * Math.sin(2 * Math.PI * 40 * i / rate) + 0.25 * Math.sin(2 * Math.PI * 800 * i / rate));
    const asset = {
        controls: [],
        mappings: [],
        tracks: [{
            gain: 1, pan: 0, highPassHz: 160,
            clips: [{source: "tone", start: 0, trim: 0, length: 1, duration: 1, gain: 1, pan: 0}],
        }],
    };
    const {samples} = mixAudio(asset, {tone: {rate, samples: [frames]}}, 1);
    const amplitude = frequency => {
        let real = 0;
        let imaginary = 0;
        for (let index = rate / 2; index < rate; index++) {
            const angle = 2 * Math.PI * frequency * index / rate;
            real += samples[0][index] * Math.cos(angle);
            imaginary += samples[0][index] * Math.sin(angle);
        }
        return 4 * Math.hypot(real, imaginary) / rate;
    };
    assert.ok(amplitude(40) < 0.02);
    assert.ok(amplitude(800) > 0.24);
});
