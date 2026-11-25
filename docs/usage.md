# ATK Audio usage guide

[Installation and quickstart](../README.md)

## Quickstart

In a fresh project directory, generate the included sound collection and render a short WAV:

```powershell
atk-audio-cli --session audio-validation-suite --validation-suite
'{"command":"session_render","seconds":4}' | atk-audio-cli --session audio-validation-suite --output audio-preview.wav
atk-audio
```

Open the viewer URL printed at startup, use the folder button to select **Audio Validation Suite**, and press **Play**. Use **Solo** to audition one track at a time. The CLI saves your session under `.audio/`.

## Open the viewer

Run `atk-audio` from your project directory and open the viewer URL printed at startup in a browser with AudioWorklet support. Press **Ctrl+P** to choose a saved session or published asset. The validation suite below creates a session you can audition.

`atk-audio` reads saved sessions and assets from the current directory. To view another directory, run `atk-audio "D:\Game Audio"` or pass the same path with `--project`. Use `--port PORT` to change the port. The viewer's playback controls are local to the browser. Make project edits with `atk-audio-cli`. The existing `atk-audio-cli --serve` command also starts the Rust viewer.

## Agent workflow

Choose a distinct session ID for each agent. Start from an asset ID to get its current published revision or create a new draft. Pass JSON via stdin or use a JSON file with **--file**. Each command returns a JSON line. Arrays execute in order, and a failure stops the batch with earlier successful commands saved.

~~~powershell
'{"command":"session_start","asset":"pickup","name":"Pickup chime"}' | atk-audio-cli --project . --session agent-a
'{"command":"session_inspect"}' | atk-audio-cli --project . --session agent-a
'{"command":"session_publish"}' | atk-audio-cli --project . --session agent-a
~~~

A session owns one editable draft. Publishing updates that asset using a revision check. If the asset advanced from a different session, the publish fails and preserves the draft. The workspace picker lists both sessions and published assets. The viewer checks for saved edits automatically and applies them when playback is idle. If an edit arrives while previewing, stop playback to see it immediately after the next check. Project files and source audio live under **.audio/**.

### Command reference

All times are seconds. IDs use lowercase letters, digits, underscores, and hyphens.

| Command | Fields besides command |
| --- | --- |
| session_start | asset, optional name |
| session_inspect, session_publish | Session from --session |
| project_list | None |
| asset_inspect | asset |
| asset_name | name |
| source_generate | id, optional instrument and strike, or wave (tone, noise, additive, or formant). Shared options include gain, seed and prompt. Tone, noise, and additive take seconds, attack, release, and optional am. Tone and noise take frequency and endFrequency. Tone takes harmonics, additive takes partials, and formant takes segments. |
| source_import | id, location (local file or direct HTTP(S) WAV URL), optional license, attribution, prompt |
| melody_import | id, location (WAV path or URL), optional name and start. Adds a synthesized melody track and clip to the current draft. |
| capture_system | Windows only. Optional seconds (0.25–30, default 15) and device (a Windows playback endpoint ID). Requires --output path.wav. |
| track_add, track_edit | id, optional name, gain, pan, mute, solo, lowPassHz, highPassHz (30–20,000 Hz, or null to clear) |
| clip_add | id, track, source, optional name, start, trim, length, duration, gain, pan, fadeIn, fadeOut, loop, trigger |
| clip_edit | id and any edited clip fields, including track to move the clip. Use trigger: null to clear it. |
| clip_split | id, newId, time on the timeline. Split fixed clips outside their fades. |
| clip_remove | id |
| control_add | id, type (slider or trigger), optional name. Slider fields are min, max, value. |
| mapping_set | control, clip, target (gain or pitch), min, max. Optional inputMin and inputMax limit the response to part of the slider range. For gain, combine retains other slider mappings and curve accepts linear or equal-power. |
| preset_set | id, seconds, optional name, sustain, lanes, and triggers. Existing IDs are updated in place. |
| preset_remove | id |
| session_render | Optional seconds and sustain, or preset. Requires --output path.wav. |
| spectrogram | scope (mix, track, clip, source, or wav), id for track/clip/source, optional asset for a published asset, location for a standalone WAV. Requires --output path.png. |
| audio_analyze | Same selection as spectrogram, plus format (waveform, features, mel, pcm, or impulse). Waveform requires --output path.png. JSON can go to stdout or --output path.json. |

Clip **trim** is the source offset and **length** is the source window. **Duration** is the maximum timeline playback time. A clip’s **loop** setting repeats its source window. The viewer’s **Loop** checkbox repeats the entire arrangement or preset from the beginning. Trigger clips start when their assigned control fires. Pitch changes playback rate. Gain and pan are linear, with pan from −1 to +1.

A track’s **highPassHz** and **lowPassHz** filters run after clip playback and pitch changes. They attenuate frequencies below and above their respective cutoffs, which stay fixed as the clip pitch changes.

To synthesize speech-like sounds in the CLI, use `source_generate` with `wave: "formant"`. Each segment takes `seconds`, optional `formants` (resonance frequencies in Hz), optional `frequency` and `endFrequency` (voice pitch in Hz), and optional `noise` and `gain` from 0 to 1. Use formants for voiced sounds, noise for breath or consonants, and a segment with only a duration for silence. The optional source-level `transition`, in seconds from 0 to 0.06, blends consecutive segments. The source stores the segment settings for inspection.

For a crossfade, map one clip's gain from 1 to 0 and the other from 0 to 1 across the same slider range. Use **curve: "equal-power"** on both clips for a power-based fade. With **combine: true**, the new gain mapping multiplies with the clip’s other gain mappings. Slider values outside **inputMin** and **inputMax** hold the corresponding endpoint.

### Instrument voices

Use `instrument` on `source_generate` to play a muted string (`muted-pluck`), a mellow bass (`rounded-bass`) or a ringing cup (`ceramic-tap`). For “play A4 softly on a muted string,” send this command in an active draft:

```json
{"command":"source_generate","id":"soft-a4","instrument":"muted-pluck","frequency":440,"strike":0.15}
```

Place the source with `clip_add`. Strike runs from 0 (soft) to 1 (firm), defaults to 0.5, and changes attack time and upper-partial strength. Use gain for level. Each voice rings for 1.5 seconds by default. Shorten it with `seconds`, or override `attack` and `release` in seconds. The source provenance records the instrument, strike and resolved additive settings.

## Build the validation suite

```powershell
atk-audio-cli --session audio-validation-suite --validation-suite
```

The command creates and publishes **Audio Validation Suite** using the standard commands defined in [validation-suite.json](../src/fixtures/validation-suite.json). It lays out 11 synthesized references as separate tracks. Play them in order or use Solo to isolate a track.

| Category | Tracks and start times |
| --- | --- |
| Tonal | Sine wave (0 s), bell (2 s), plucked string (6 s) |
| Transient | Hand clap (10 s), metal impact (12 s), water drop (15 s) |
| Mechanical | Electric fan (17 s), gear mechanism (32 s) |
| Environmental | Rain (37 s), wind (47 s), ocean waves (58 s) |

Use `wave: "additive"` to combine up to 32 sine partials with independent `frequency`, `endFrequency`, `gain`, `phaseDegrees`, and `decay`. Frequencies are in Hz and `decay` is the exponential time constant in seconds. The sum is scaled when the partial gains add up to more than 1. Tone, noise, and additive sources can also take `am: {frequency, depth}`, where frequency is in Hz and depth is from 0 to 1.

Measure the fan’s amplitude modulation over its section of the timeline with this command.

```powershell
atk-audio-cli --session audio-validation-suite --command '{"command":"audio_analyze","scope":"track","id":"electric-fan","start":18,"end":22,"hopMs":10}'
```

## Playback presets

Playback presets store control automation with the asset. An agent can author a preset with **preset_set**, giving each slider a lane of time and value keyframes. Slider values interpolate linearly between keyframes and hold the first or last value outside them. A trigger fires once at its specified time. Controls without lanes use their saved defaults.

In the viewer, open **Choose playback** in the bottom transport, select a preset, and click **Play**. Choose **Arrangement** to return to manual playback. The sliders show the automated values during preset playback and return to their previous manual settings when it ends. Pass a saved preset ID in the **preset** field of **session_render** or **spectrogram**.

Use **Mute** and **Solo** beside a track to audition layers during manual or preset playback. Multiple soloed tracks play together, and mute takes precedence on an individual track. **Reset Mute/Solo** restores the saved track settings. These controls affect browser playback. WAV exports use the saved track mix.

WAV import supports standard RIFF PCM and 32-bit float, mono or stereo, at 8–192 kHz. Individual source files and exports are limited to 120 seconds. Exports are stereo 48 kHz, 16-bit PCM.

### Turn singing into a track

In the viewer, open **Record audio** with the microphone button. Choose **Microphone**, start recording, and sing or hum a single melody for up to 30 seconds. Stop, preview the take, and choose **Download WAV**. Share the recording with Codex to add it to your draft, or import it directly into an existing draft session.

```powershell
'{"command":"melody_import","id":"sung-melody","location":"C:/path/to/sung-tune.wav"}' | atk-audio-cli --project . --session my-draft
```

The import estimates pitches, snaps them to semitones, and synthesizes a tone melody with the detected timing and rests. The detected notes are available in the source provenance. Open the draft in the viewer to audition the new track.

### Capture a Windows desktop app

When browser sharing is unavailable, record the Windows default playback device from a terminal. Start playback in the desktop app when the recorder prints its status. Use a new output path for each take.

```powershell
'{"command":"capture_system","seconds":15}' | atk-audio-cli --output desktop-take.wav
```

The app must play through the captured endpoint in shared mode. If it uses a different playback endpoint, pass its Windows endpoint ID as the command's `device` field. The WAV can be imported with `source_import` or analyzed with `audio_analyze`.

## Inspect audio with a spectrogram

Use the returned PNG with a vision-capable agent. The horizontal axis shows seconds, the vertical axis shows frequency in hertz, and brighter colors indicate stronger frequencies. Images use fixed frequency and level scales by default so you can compare revisions.

```powershell
'{"command":"spectrogram","scope":"source","id":"sine-reference"}' | atk-audio-cli --session audio-validation-suite --output sine-source.png
'{"command":"spectrogram","scope":"clip","id":"sine-reference-1"}' | atk-audio-cli --session audio-validation-suite --output sine-clip.png
'{"command":"spectrogram","scope":"track","id":"sine","start":0.2,"end":0.8,"maxHz":600}' | atk-audio-cli --session audio-validation-suite --output sine-track.png
'{"command":"spectrogram","scope":"mix","end":2.5}' | atk-audio-cli --session audio-validation-suite --output validation-mix.png
'{"command":"spectrogram","scope":"wav","location":"mix.wav"}' | atk-audio-cli --output exported-mix.png
```

Source and clip times are relative to their own start. Track and mix times follow the timeline. A track or clip is inspected in isolation with its gain and pan applied. An isolated trigger clip fires at zero unless you supply an explicit trigger event. Use **events** to script slider changes and trigger events for a rendered clip, track, or mix. **seconds** sets the render duration, and **sustain** keeps untriggered loops running through that duration. Use **start** and **end** to crop the result.

The default display uses a logarithmic frequency axis, 30 Hz to 16 kHz (capped at half the sample rate), and −85 to 0 dBFS. Change **minHz**, **maxHz**, **floorDb**, **ceilingDb**, **scale** (log or linear), or **fftSize** as needed. A larger FFT resolves nearby pitches more clearly. A smaller FFT gives sharper timing for short transients.

### Other analysis outputs

```powershell
'{"command":"audio_analyze","format":"waveform","scope":"clip","id":"sine-reference-1"}' | atk-audio-cli --session audio-validation-suite --output sine-waveform.png
'{"command":"audio_analyze","format":"features","scope":"mix","end":2.5,"hopMs":100}' | atk-audio-cli --session audio-validation-suite --output validation-features.json
'{"command":"audio_analyze","format":"mel","scope":"mix","end":2.5,"melBands":48}' | atk-audio-cli --session audio-validation-suite --output validation-mel.json
'{"command":"audio_analyze","format":"pcm","scope":"source","id":"sine-reference","start":0.01,"end":0.025}' | atk-audio-cli --session audio-validation-suite
```

- **waveform** draws a min/max amplitude envelope for each channel on a fixed full-scale axis. Orange marks samples beyond full scale in a rendered mix.
- **features** returns timestamped peak and RMS levels, frequency-band powers in dBFS, centroid in hertz, spectral flatness, and positive spectral flux. The numeric band boundaries accompany the data. Peak uses the largest channel magnitude and RMS averages channel power. Flux measures increases in the normalized magnitude spectrum relative to the previous window. The first flux value and the centroid and flatness of a silent FFT window are null.
  - **pitch** estimates the dominant fundamental from 25 to 1,000 Hz using autocorrelation, with a periodicity score and up to eight FFT harmonic peaks when the bins can resolve them. Frames without a clear period return null.
  - **onsets** reports detected level rises with onset time, attack duration, and local RMS peak using a 2 ms envelope. Attack duration is measured from the detected start to within 1 dB of that peak.
  - **modulation** reports a 5th-to-95th percentile depth and an estimated repeat rate for pitch, RMS level, and spectral centroid. Rate is null when the trace has no clear repeat in the measurable range. Use a shorter hop to measure faster changes and a range spanning several cycles.
  - **stereo** reports zero-lag correlation, the fraction of mid/side energy in the side signal, and an estimated interchannel delay up to ±20 ms when correlation is strong enough. Positive delay means the right channel lags. Mono returns null. Periodic sounds can have multiple plausible delay matches.
- **impulse** measures a supplied impulse response. It returns the time and level of the strongest sample and extrapolates decay time from the reverse-integrated energy between −5 and −25 dB (`t20Seconds`) and −5 and −35 dB (`t30Seconds`). A missing decay interval returns null. Select a clean recording of the impulse and its tail, since background noise or truncation can skew the estimate.
- **mel** returns a log-mel power matrix in dBFS, with frame times and band centers in hertz. Each row is a time window and each column is a triangular mel filter. Set **melBands** from 8 to 96 or use the default of 40.
- **pcm** returns one float array per channel with the sample rate and start frame. Use **start** and **end** to select up to 8192 frames. Rendered floats are taken before WAV clipping or quantization.

Features and mel accept **hopMs** (default 50 ms), **fftSize** (default 4096), and **maxHz** (default 16 kHz or Nyquist, whichever is lower). Mel also accepts **minHz**. Levels are floored at −120 dBFS. The FFT window is centered on each output timestamp and can include neighboring samples outside the selected range. Use matching FFT window lengths in seconds, hop sizes, and frequency limits when comparing assets at different sample rates. Select a shorter range or increase **hopMs** if an analysis would exceed 3000 windows.

## Verify

Run `cargo test --locked` to build the CLI and check it alongside the read-only viewer. Then run `node tests/project.test.mjs` for regression tests against the built CLI, native audio engine, and browser mixer. Set `ATK_AUDIO_TEST_CLI` to use a binary outside `target/debug`.

## CLI command conventions

Send a JSON object through stdin, pass it with `--command JSON`, or read it with `--file FILE` or `--input FILE`. Send `{"command":"help"}` for command discovery.

Send a JSON array of 1 to 1,024 commands to run a batch. Use `--batch` to require array input. The CLI checks the command shape before it starts. Commands then run in order, with one JSON reply per attempted command. Execution stops at the first failure and earlier edits remain applied. Check the exit status and JSON replies before retrying.

Use `{"command":"help","all":true}` to list documentation topics. Read a topic with `{"command":"help","topic":"command-api"}` and pass the returned `next_offset` as `offset` until it is null.

Successful command replies include `ok: true` and a `result`. Failures exit with status 1 and report JSON on stdout. Standard export replies include `written.path` and `written.bytes` for requested output files.

Run commands locally through the CLI. If publishing reports a revision conflict, inspect the saved draft and current published asset before retrying.

## Debug recordings and replay

Add `--debug` to a CLI command or batch to record it. The CLI prints the recording path to stderr and keeps command replies on stdout.

```powershell
'{"command":"session_inspect"}' | atk-audio-cli --session my-draft --debug
atk-audio-cli --replay PATH_TO_RECORDING.jsonl
```

Each invocation creates a JSONL file under `PROJECT/.audio/recordings/`. The file contains the initial saved state and a request, reply, and saved-state snapshot for each attempted command. A request without a reply means the outcome was not recorded. Inspect the saved project before retrying it.

Replay reads the recorded timeline without executing commands or restoring state. Add `--frame N` to print a single saved snapshot. Frame 0 is the initial state. Snapshots reflect saved data observed by the CLI, so another CLI writing concurrently can affect what a snapshot contains. Recording files contain the full request and saved-state data.
