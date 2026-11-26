# atk-audio

- Keep the Cargo package responsible for the command interface, project validation, and all writes to `.audio/`.
- Use `src/cli.rs` for JSON input, command dispatch, and output files. Keep project persistence and edit rules in the Rust library.
- Keep `atk-audio` and `web/` read-only. The server serves bundled assets, saved JSON, and source WAVs. The browser owns only transient audition and presentation state.
- Keep native audio processing in `src/audio/`. Return audio and metadata to the command layer for persistence.
- Preserve existing `.audio/` documents and command JSON when changing internals. Coordinate concurrent edits and keep publication revision checks.
- Use `cargo test --locked` for the binaries, then run `node tests/project.test.mjs` for CLI, audio engine, and browser mixer regression checks. Exercise writes in an isolated project.
