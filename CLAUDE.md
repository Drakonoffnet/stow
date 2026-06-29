# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build               # debug
cargo build --release     # release

# Run (debug GUI)
cargo run

# Test
cargo test                # all tests (unit + integration)
cargo test <name>         # single test by name, e.g. `cargo test builds_name_with_timestamp`
cargo test --test engine_e2e   # one integration test file

# macOS .app bundle
./scripts/bundle_macos.sh             # host arch → dist/Stow.app
./scripts/bundle_macos.sh --universal # fat binary (arm64 + x86_64)
```

## Architecture

The project is split into a headless library and a GUI binary that consumes it.

### Library (`src/core/`)

All logic lives here; it has no dependency on egui.

- **`engine.rs`** — the central coordinator. Spawns a `rayon`-backed worker pool (`Config::jobs`, defaulting to CPU count). Callers submit `JobSpec`s; the engine enqueues them as `Job`s and emits `Event`s (`Accepted`, `Progress`, `Finished`, `Log`) over a `crossbeam_channel`. After every emit it calls a `wake: Arc<dyn Fn()>` callback so the GUI can request a repaint without polling.

- **`archive.rs`** — parallel ZIP writer. Files are compressed in parallel via `rayon` (each file is a raw-deflate task), then the ZIP container is assembled sequentially. The ZIP is hand-written (no external zip crate): local headers → central directory → EOCD. UTF-8 filenames are flagged (`0x0800`). **Limitation**: no ZIP64, so the archive and individual files must stay under 4 GB. Empty directories are not preserved.

- **`transfer.rs`** — the `Destination` trait and its two backends. Every backend follows a two-phase write: `stage()` returns a local `.part` path to write into, then `finalize()` moves it atomically (local → `fs::rename`; SSH → SFTP upload to a `.part` remote name, then remote rename). `LocalDestination` writes to a local/mounted path; `SshDestination` connects via `ssh2` (vendored OpenSSL), verifies the host key against `~/.ssh/known_hosts` (mismatches are hard errors; unknown hosts are accepted for the session without writing back — trust-on-first-use), then uploads over SFTP.

- **`secret.rs`** — thin wrapper around the `keyring` crate. SSH passwords and key passphrases are never stored in `JobSpec`; the spec holds only a keychain account reference string (`"user@host:port"` or `"user@host:port/key"`). Secrets are written to and read from macOS Keychain at submission/upload time.

- **`naming.rs`** — generates archive names: `<folder_name>_YYYY-MM-DD_HHMMSS.zip`.

- **`model.rs`** — all shared data types: `JobId`, `JobSpec`, `JobStatus`, `DestinationSpec`, `SshConfig`, `SshAuth`, `Config`.

- **`error.rs`** — `CoreError` enum (thiserror). `Canceled` is a first-class variant so cancellation is distinguishable from failures.

### GUI binary (`src/main.rs` + `src/theme.rs`)

An `eframe::App` named `App`. On construction it starts the `Engine` and wires `ctx.request_repaint` as the `wake` callback. Each `update()` call drains the event channel first (`drain_events()`), then renders the window.

The UI has two main columns (source list / destination panel), an actions row (checkboxes + Start button), a jobs list with per-job progress bars, and a log panel. Layout switches from two-column to stacked below 660 px.

Drag-and-drop routing: the app tracks the hover position during a drag (`drop_pos`) because the OS may clear the pointer on the drop frame. The stored position is compared against `source_rect` and `dest_rect` to decide whether a dropped folder goes to the source list or the destination.

`theme.rs` defines the palette constants and provides typed text helpers (`reg`, `med`, `semi`, `bold`, `extra`, `mono`) that all return `egui::RichText`. Fonts (Manrope at five weights) are embedded at compile time via `include_bytes!` and registered as named `FontFamily` variants.

### Tests

- Unit tests are inline inside `src/core/archive.rs` and `src/core/transfer.rs`.
- `tests/engine_e2e.rs` — full round-trip through `Engine`: submit a job, wait for `Event::Finished`, assert the archive exists and checksum is correct.
- `tests/zip_interop.rs` — uses the system `unzip` and (on macOS) `ditto` to verify that the hand-written ZIP is valid and that Cyrillic filenames round-trip correctly.
