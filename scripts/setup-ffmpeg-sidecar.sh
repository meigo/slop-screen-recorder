#!/usr/bin/env bash
# Creates dummy sidecar placeholders for local development.
# In local dev, the app falls back to system FFmpeg from PATH.
# CI replaces these with real FFmpeg binaries before building.

set -euo pipefail
DIR="$(cd "$(dirname "$0")/../src-tauri/binaries" && pwd)"
mkdir -p "$DIR"

touch "$DIR/ffmpeg-x86_64-pc-windows-msvc.exe"
touch "$DIR/ffmpeg-aarch64-apple-darwin"
touch "$DIR/ffmpeg-x86_64-apple-darwin"
touch "$DIR/ffmpeg-x86_64-unknown-linux-gnu"

echo "Created dummy sidecar placeholders in $DIR"
