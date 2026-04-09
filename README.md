# Slop Screen Recorder

Minimalistic cross-platform screen recorder built with Tauri v2, SvelteKit, and Rust.

Wraps FFmpeg with a clean native UI — select a source, hit record, get an MP4. No bloat, no accounts, no cloud.

## Features

- **Cross-platform** — macOS, Windows, and Linux
- **Hardware-accelerated encoding** — H.264 via VideoToolbox (macOS), with fallback to libx264
- **Audio capture** — optional microphone recording
- **Configurable** — choose source, FPS (15/24/30/60), and output directory
- **Tiny footprint** — native Tauri app, no Electron/Chromium bundle

## Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) 1.77+
- [FFmpeg](https://ffmpeg.org/)

### Installing FFmpeg

**macOS:**

```sh
brew install ffmpeg
```

**Windows:**

Download from [ffmpeg.org](https://ffmpeg.org/download.html) and add to PATH. Or via [Chocolatey](https://chocolatey.org/):

```sh
choco install ffmpeg
```

**Linux (Debian/Ubuntu):**

```sh
sudo apt install ffmpeg
```

## Getting Started

```sh
# install dependencies
npm install

# run in development mode
npm run tauri dev

# build for production
npm run tauri build
```

## Scripts

| Command | Description |
|---|---|
| `npm run tauri dev` | Start the app in development mode with hot reload |
| `npm run tauri build` | Build a production binary/installer |
| `npm test` | Run all checks (lint + type check + clippy + rust tests) |
| `npm run lint` | Run ESLint |
| `npm run lint:fix` | Run ESLint with auto-fix |
| `npm run check` | Run svelte-check (TypeScript + Svelte type checking) |
| `npm run test:rust` | Run cargo clippy and cargo test |

## Project Structure

```
slop-screen-recorder/
├── src/                    # SvelteKit frontend
│   └── routes/
│       └── +page.svelte    # Main recording UI
├── src-tauri/              # Tauri + Rust backend
│   └── src/
│       ├── lib.rs          # Tauri app setup and command registration
│       └── recorder.rs     # FFmpeg process management and platform logic
├── eslint.config.js        # ESLint config (Svelte + TypeScript)
├── svelte.config.js        # SvelteKit config (static adapter for SPA)
└── package.json
```

## How It Works

The Rust backend spawns FFmpeg as a child process with platform-specific capture arguments:

| Platform | Capture | Encoder |
|---|---|---|
| macOS | `avfoundation` | `h264_videotoolbox` (hardware) |
| Windows | `gdigrab` | `libx264` |
| Linux | `x11grab` | `libx264` |

Recording starts by sending the appropriate FFmpeg command and stops by writing `q` to stdin for a graceful shutdown that finalizes the MP4 file.

## Tech Stack

- [Tauri v2](https://v2.tauri.app/) — native desktop runtime
- [SvelteKit](https://svelte.dev/docs/kit) — frontend framework (static SPA mode)
- [Rust](https://www.rust-lang.org/) — backend logic
- [FFmpeg](https://ffmpeg.org/) — screen capture and video encoding

## License

[MIT](LICENSE)
