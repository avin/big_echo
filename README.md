# BigEcho

Standalone desktop recorder (Tauri target) for macOS and Windows.

## MVP scope

- Record `system + microphone` into single `.opus` file
- Save sessions by path: `<tag>/DD.MM.YYYY/meeting_HH-mm-ss/`
- Upload audio to transcription cloud API
- Send transcript to summary cloud API
- Save `audio.opus`, transcript, summary and `meta.json`
- Control recording from tray and main window
- Configure API URLs and keys in Settings window

## Current status in this repository

This directory contains the initial implementation skeleton:

- Rust domain/storage/settings modules
- Russian date path formatter and tests
- Session metadata file writer
- Settings validation and JSON persistence stubs

## Local prerequisites for full build

- Rust toolchain (`cargo`)
- Tauri prerequisites for your OS
- Node.js 18+

## Next run commands (on machine with Rust + Tauri)

```bash
cd BigEcho
npm install
npm run tauri dev
```

## Build release installers

All commands below are run from `BigEcho/`.

### 1) Install dependencies

```bash
cd BigEcho
npm install
```

### 2) Build for macOS

Build for Apple Silicon:

```bash
rustup target add aarch64-apple-darwin
npm run tauri build -- --target aarch64-apple-darwin
```

Build for Intel:

```bash
rustup target add x86_64-apple-darwin
npm run tauri build -- --target x86_64-apple-darwin
```

Build for current macOS host architecture:

```bash
npm run tauri build
```

Artifacts are created in:

```text
src-tauri/target/<target>/release/bundle/
```

Typical macOS artifacts:
- `.app`
- `.dmg`

### 3) Build for Windows

On a Windows machine (MSVC toolchain):

```bash
rustup target add x86_64-pc-windows-msvc
npm run tauri build -- --target x86_64-pc-windows-msvc
```

Artifacts are created in:

```text
src-tauri/target/x86_64-pc-windows-msvc/release/bundle/
```

Typical Windows artifacts:
- `.msi`
- NSIS installer (`.exe`) if NSIS is available in your environment

### 4) Useful build options

Build without installer bundle (binary only):

```bash
npm run tauri build -- --no-bundle
```

Build with verbose logs:

```bash
npm run tauri build -- --verbose
```
