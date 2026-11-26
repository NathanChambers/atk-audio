---
name: atk-audio
description: Use atk-audio to create, edit, audition, analyze, or render audio in a local project. Apply when the user invokes $atk-audio or asks to work with atk-audio sessions, assets, or the browser viewer.
---

# atk-audio

Use `atk-audio-cli --project DIR --session ID` when choosing or switching targets during a task. Use MCP tools already attached to the intended project and session when available. Follow an explicit user preference for either transport. If atk-audio cannot perform a required operation, explain the limitation and ask before switching to external authoring.

- Check `atk-audio-cli --help` for commands and `atk-audio --help` for viewer options. Use the source checkout README for installation and the command reference.
- Run `atk-audio-cli` from the intended project directory or pass `--project DIR`. Use `project_list` to find saved assets and sessions. Use a distinct session ID for a new draft.
- When configuring a stdio MCP host, launch `atk-audio-cli --mcp --project DIR --session ID`. Use a separate process for each MCP target.
- Pass JSON through stdin or `--file FILE`. In PowerShell, use `'{"command":"project_list"}' | atk-audio-cli --project .`.
- Start a draft with `session_start` and its asset ID. Pass `--session ID` for draft operations. Inspect the draft with `session_inspect` before editing an existing session.
- Batch commands with known inputs using MCP `batch` or a CLI JSON array. Split batches when a later command depends on a returned result. Earlier successful edits are saved when a later command fails.
- Use `session_render` for WAV exports and `spectrogram` or waveform analysis for PNG inspection. Supply a fresh path as MCP `output_path` or CLI `--output FILE`.
- Publish with `session_publish` when publication is requested. If another session advanced the asset revision, inspect the current asset before deciding how to reconcile the draft.
- When the user wants browser playback, run `atk-audio` from the project directory or pass the directory as a path, such as `atk-audio "D:/Game Audio"`.
- Use `atk-audio --project DIR` for the equivalent named option. Add `--port PORT` when a different port is needed.
- Share the URL printed by the read-only viewer. The CLI and MCP authoring commands work while the viewer is closed.
