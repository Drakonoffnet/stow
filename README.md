# Stow

A macOS desktop app that packs folders into ZIP archives and moves them to a destination — local, network, or remote over SSH/SFTP.

Drop folders into the source zone, pick a destination, press **Start**. Each folder becomes a separate `.zip` named `<folder>_YYYY-MM-DD_HHMMSS.zip`. Files inside each archive are compressed in parallel using all available CPU cores; multiple archives are processed concurrently.

## Features

- Drag-and-drop or dialog-based folder selection
- Local/mounted network destinations and SSH/SFTP targets
- SSH authentication via agent, password, or key file — credentials stored in macOS Keychain
- Per-job progress with cancellation
- Optional SHA-256 checksum of the finished archive
- Optional source removal after successful transfer
- Atomic delivery: the archive appears at the destination only when complete

## Requirements

macOS 11+. Builds for Apple Silicon and Intel (universal binary optional).

## Build & run

```bash
cargo run                              # debug GUI
cargo build --release                  # release binary
./scripts/bundle_macos.sh             # dist/Stow.app (host arch)
./scripts/bundle_macos.sh --universal # dist/Stow.app (arm64 + x86_64)
```

## Limitations

- Standard ZIP (no ZIP64): archives and individual files must be under 4 GB
- Empty directories are not preserved in the archive
- SSH host keys: unknown hosts are accepted on first connection without persisting to `known_hosts`
- Not notarized — ad-hoc signed for local use only
