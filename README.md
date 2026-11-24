# ATK Audio

Design and analyze sound effects with an AI agent. Audition saved arrangements in a local browser viewer.

## What you can do

- Synthesize sounds, import WAV sources, and arrange clips on tracks.
- Audition layers with mute and solo, adjust controls live, and preview saved automation presets.
- Export stereo WAV files and inspect waveforms, spectrograms, or structured audio measurements.

## Install

Install a stable Rust toolchain and Node.js 20 or newer. The browser viewer uses AudioWorklet for playback. From the atk-audio source checkout, run:

```sh
cargo install --path . --locked
```

Cargo installs `atk-audio-cli` for your agent and `atk-audio` for the browser viewer. Both go in Cargo's bin directory, usually `~/.cargo/bin`.

## Use with an agent

Add the [`atk-audio` skill](.agents/skills/atk-audio/SKILL.md) using your agent's skill installation instructions. Open the agent in your project directory and give it a task.

> Create a short UI confirmation sound, show me a spectrogram, and export it as WAV.

## Open the viewer

Run `atk-audio` from your project directory and open the [Audio viewer](http://127.0.0.1:7880). Once your agent has saved a workspace, choose it with the folder button. From there, press **Play** to hear the arrangement and use **Solo** to audition individual tracks.

## Learn more

- [Usage and command reference](docs/usage.md) for draft sessions, publishing, analysis, presets, and debug recordings.
- [Validation suite](src/fixtures/validation-suite.json) for examples of synthesized sounds and track arrangements.

## Related tools

- [ATK Mesh](https://github.com/NathanChambers/atk-mesh)
- [ATK UI](https://github.com/NathanChambers/atk-ui)

## License

[MIT](LICENSE)
