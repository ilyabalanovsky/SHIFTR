# SHIFTR

SHIFTR is a Tauri 2 desktop utility for local batch conversion of video, audio, and image files.

## Stack

- Tauri 2 + Rust
- React + TypeScript + Vite
- FFmpeg/ffprobe integration for video and audio
- Rust `image` pipeline for image conversions

## Development

```powershell
npm install
npm run tauri:dev
```

If the current terminal cannot find Cargo after a fresh Rust install:

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
npm run tauri:dev
```

## Build

```powershell
npm run build
npm run tauri:build
```

The Windows build produces:

- `src-tauri/target/release/shiftr.exe`
- `src-tauri/target/release/bundle/msi/SHIFTR_0.1.0_x64_en-US.msi`
- `src-tauri/target/release/bundle/nsis/SHIFTR_0.1.0_x64-setup.exe`

## FFmpeg

The app looks for FFmpeg in this order:

1. A path selected in the app settings.
2. Bundled binaries under `src-tauri/resources/bin/`.
3. `ffmpeg`/`ffprobe` on the system PATH.

For the planned LGPL-safe bundled release, place matching binaries here before packaging:

- `src-tauri/resources/bin/ffmpeg.exe`
- `src-tauri/resources/bin/ffprobe.exe`

## Verification

```powershell
npm run lint
npm run build
cd src-tauri
cargo test
```
