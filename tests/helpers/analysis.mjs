import {execFileSync} from "node:child_process";
import {mkdtempSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join, resolve} from "node:path";
import {fileURLToPath} from "node:url";

export function analyzeAudio({samples, rate}, options) {
    const binary = process.platform === "win32" ? "atk-audio-cli.exe" : "atk-audio-cli";
    const cli = process.env.ATK_AUDIO_TEST_CLI ? resolve(process.env.ATK_AUDIO_TEST_CLI) : fileURLToPath(new URL("../../target/debug/" + binary, import.meta.url));
    const project = mkdtempSync(join(tmpdir(), "audio-analysis-"));
    try {
        const channels = samples.length;
        const frames = samples[0].length;
        const bytes = Buffer.alloc(44 + frames * channels * 4);
        bytes.write("RIFF");
        bytes.writeUInt32LE(bytes.length - 8, 4);
        bytes.write("WAVEfmt ", 8);
        bytes.writeUInt32LE(16, 16);
        bytes.writeUInt16LE(3, 20);
        bytes.writeUInt16LE(channels, 22);
        bytes.writeUInt32LE(rate, 24);
        bytes.writeUInt32LE(rate * channels * 4, 28);
        bytes.writeUInt16LE(channels * 4, 32);
        bytes.writeUInt16LE(32, 34);
        bytes.write("data", 36);
        bytes.writeUInt32LE(bytes.length - 44, 40);
        for (let frame = 0; frame < frames; frame++) {
            for (let channel = 0; channel < channels; channel++) {
                bytes.writeFloatLE(samples[channel][frame], 44 + (frame * channels + channel) * 4);
            }
        }
        const location = join(project, "signal.wav");
        writeFileSync(location, bytes);
        const input = JSON.stringify({command: "audio_analyze", scope: "wav", location, ...options});
        return JSON.parse(execFileSync(cli, ["--project", project], {input, encoding: "utf8"})).result;
    } finally {
        rmSync(project, {recursive: true, force: true});
    }
}
