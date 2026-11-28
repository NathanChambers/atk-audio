# atk-audio

Design and analyze sound effects with an AI agent. Audition saved arrangements in a local browser viewer.

## What you can do

- Synthesize sounds, import WAV sources, and arrange clips on tracks.
- Audition layers with mute and solo, adjust controls live, and preview saved automation presets.
- Export stereo WAV files and inspect waveforms, spectrograms, or structured audio measurements.

## Install

Install a stable Rust toolchain. The browser viewer uses AudioWorklet for playback. From the atk-audio source checkout, run:

```sh
cargo install --path . --locked
```

Cargo installs `atk-audio-cli` for your agent and `atk-audio` for the browser viewer. Both go in Cargo's bin directory, usually `~/.cargo/bin`.

## Use with an agent

Add the [`atk-audio` skill](.agents/skills/atk-audio/SKILL.md) using your agent's skill installation instructions. Open the agent in your project directory and give it a task.

> Create a short UI confirmation sound, show me a spectrogram, and export it as WAV.

## Open the viewer

Run `atk-audio` from your project directory. Open the printed URL to see a picker for saved assets and sessions. Choose one, then press **Play** to hear the arrangement and use **Solo** to audition individual tracks.

To browse the read-only viewer from another device on a trusted local network, run `atk-audio --lan`. Open `http://<host-LAN-IP>:<port>` on the other device using the host’s IPv4 address and the port printed at startup. The ordinary HTTP LAN URL lets you browse saved sessions and assets. In-browser auditioning uses AudioWorklet and requires a secure browser context, such as the printed loopback URL on the host or a trusted HTTPS origin.

## Learn more

- [Usage and command reference](docs/usage.md) for draft sessions, publishing, analysis, presets, and debug recordings.
- [Validation suite](src/fixtures/validation-suite.json) for examples of synthesized sounds and track arrangements.

## Related tools

- [atk-mesh](https://github.com/NathanChambers/atk-mesh)
- [atk-ui](https://github.com/NathanChambers/atk-ui)

## License

[MIT](LICENSE)
