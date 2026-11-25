---
name: atk-audio
description: Use the Audio Workbench CLI to create, edit, audition, analyze, or render audio in a local project. Apply when the user invokes $atk-audio or asks to work with Audio Workbench sessions, assets, or the browser viewer.
---

# atk-audio

When this skill is invoked, use the Audio Workbench CLI command workflow unless the user explicitly specifies another method. Author audio in the target project with `atk-audio-cli` commands. Scripts may prepare and submit CLI command batches. If the CLI cannot perform a required operation, explain the limitation and ask before switching to external authoring.

- Check `atk-audio-cli --help` for commands and `atk-audio --help` for viewer options. Use the source checkout README for installation and the command reference.
- Run `atk-audio-cli` from the intended project directory or pass `--project DIR`. Use `project_list` to find saved assets and sessions. Use a distinct session ID for a new draft.
- Pass JSON through stdin or `--file FILE`. In PowerShell, use `'{"command":"project_list"}' | atk-audio-cli --project .`.
- Start a draft with `session_start` and its asset ID. Pass `--session ID` for draft operations. Inspect the draft with `session_inspect` before editing an existing session.
- Check the exit status and JSON replies after a batch. Earlier successful edits are saved when a later command fails.
- Use `session_render` with `--output FILE.wav` for WAV exports. Use `spectrogram` or waveform analysis with `--output FILE.png` for visual inspection. Use a fresh output path.
- Publish with `session_publish` when publication is requested. If another session advanced the asset revision, inspect the current asset before deciding how to reconcile the draft.
- When the user wants browser playback, run `atk-audio` from the project directory or pass the directory as a path, such as `atk-audio "D:/Game Audio"`.
- Use `atk-audio --project DIR` for the equivalent named option. Add `--port PORT` when a different port is needed.
- Share the URL printed by the read-only viewer. Run edits through `atk-audio-cli`, which can operate while the viewer is closed.
