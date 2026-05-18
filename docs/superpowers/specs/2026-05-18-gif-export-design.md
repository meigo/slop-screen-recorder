# GIF Export — Design

Add an opt-in "Convert to GIF" action for a finished recording. Keeps the existing MP4 flow untouched; GIF is a post-process step the user invokes per-clip.

## Goals

- One-click GIF from the most recent MP4.
- Produce small, clean GIFs by default (no UI knobs in v1).
- Exploit the static-content nature of screen recordings: collapse unchanged stretches into long-delay frames, encode only changed regions.

## Non-goals (v1)

- Recording directly to GIF.
- Format selector / "always produce both" toggle.
- User-facing controls for width, fps, or dithering.
- Progress bar or percent-done parsing of ffmpeg output.
- Cancellation of an in-progress conversion.
- Batch conversion of older recordings (only the most recent recording, shown in the record bar, is convertible from the UI).

## User flow

1. User records as today → MP4 lands in the output directory, info row appears in the record bar with Play / Show-in-folder.
2. A new "Convert to GIF" button sits alongside Play / Show-in-folder in the existing `.output-actions` row.
3. User clicks it. The button enters a "converting…" state (spinner icon, disabled). Play / Show-in-folder remain enabled (they point at the MP4).
4. On success: a second info row appears beneath the MP4 row, showing the GIF basename + its own Play / Show-in-folder actions. The Convert button is hidden once a GIF exists for this recording.
5. On failure: an `alert()` shows the ffmpeg error (matches the existing error pattern for start/stop). Button returns to its idle state so the user can retry.
6. Starting a new recording clears the GIF state — the GIF info row goes away, the Convert button reappears on the next stop.

## Architecture

### Backend (Rust)

New Tauri command in `src-tauri/src/recorder.rs`:

```rust
#[tauri::command]
pub fn convert_to_gif(
    state: State<RecorderState>,
    input_path: String,
) -> Result<String, String>
```

- Validates that `input_path` exists and has a `.mp4` extension.
- Derives output path: same directory, same basename, `.gif` extension. If a file already exists at that path, overwrite it (mirrors existing recording behavior — `-y` is already used).
- Resolves ffmpeg via the existing `find_ffmpeg` helper.
- Spawns ffmpeg **synchronously** (blocking call, `.output()` not `.spawn()`) — no need to track it in `RecorderState`. The Tauri command runs on an async worker, so the UI stays responsive.
- Returns the GIF path on success; returns `stderr` text on non-zero exit.

Register the command in `src-tauri/src/lib.rs` alongside the existing `recorder::` handlers.

### FFmpeg invocation

Single command, single pass via `filter_complex`:

```
ffmpeg -y -i <input.mp4> \
  -filter_complex "fps=10,scale='min(720,iw)':-2:flags=lanczos,mpdecimate,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=diff_mode=rectangle" \
  -loop 0 <output.gif>
```

What each piece does:

- `fps=10` — ceiling on frame rate. Smallest per-frame delay becomes 100ms; static segments produce much longer delays via `mpdecimate`.
- `scale='min(720,iw)':-2:flags=lanczos` — cap width at 720px (only downscale; never upscale). Height auto-derived, rounded to even. Lanczos for quality.
- `mpdecimate` — drops near-identical consecutive frames. Default thresholds. Kept frames retain their original timestamps, so dropped frames become longer GIF delays automatically.
- `split[a][b]` — fork the stream for the two palette stages.
- `palettegen=stats_mode=diff` — build a palette weighted toward pixels that change between frames (pairs with `diff_mode=rectangle` below).
- `paletteuse=diff_mode=rectangle` — for each frame, only re-encode the bounding box of pixels that actually changed. Big size win for screen content with localized motion.
- `-loop 0` — infinite loop (standard for shareable GIFs).

Windows: spawn via the existing `ffmpeg_command()` helper so `CREATE_NO_WINDOW` keeps the console hidden.

### Frontend (`src/routes/+page.svelte`)

State additions:

```ts
let gifPath = $state('');
let convertingGif = $state(false);
```

Reset both to empty/false whenever a new recording starts (in `startRecording`, right where `outputPath` is reassigned later — clear at the top).

New handler:

```ts
async function convertToGif() {
	convertingGif = true;
	try {
		gifPath = await invoke<string>('convert_to_gif', { inputPath: outputPath });
	} catch (e) {
		alert(`Failed to convert to GIF: ${e}`);
	} finally {
		convertingGif = false;
	}
}
```

UI changes inside the existing `{#if outputPath && !recording}` block:

- Add a third button to `.output-actions`: "Convert to GIF". Hidden when `gifPath` is set. Disabled and shows a spinner icon when `convertingGif` is true.
- When `gifPath` is set, render a second `.output-info` row below the MP4 row showing the GIF basename and Play / Show-in-folder buttons that target `gifPath` instead of `outputPath`.
- Reuse the existing `openVideo` / `showInFolder` patterns; either parameterize them or add `openGif` / `showGifInFolder` siblings (parameterization is cleaner — refactor to take a path argument).

Icon for the convert button: `FileImage` from `@lucide/svelte` (already the icon library in use). Spinner state: reuse `RefreshCw` with a CSS `animation: spin 1s linear infinite` rule.

### Error surface

- ffmpeg not found → already guarded at app start (`ffmpegAvailable`); the convert button only appears when the rest of the UI does, so this is implicitly covered.
- ffmpeg fails mid-conversion → stderr returned as the error string, surfaced via `alert()` (matches existing pattern; users in the prereqs are expected to read alerts).
- Concurrent click while `convertingGif` is true → button is disabled.

## Testing

- `npm test` (format + lint + svelte-check + clippy + cargo test) must pass.
- Manual: record a short clip with a static region and some motion. Convert. Verify:
  - GIF file appears next to the MP4 with the same basename.
  - Static portions of the clip render as long-held frames (visible by stepping through with a viewer that shows per-frame delays, or simply by file size — should be a fraction of a naïvely-encoded GIF).
  - "Show in folder" reveals the GIF.
  - Starting a new recording hides the GIF row and re-shows the Convert button.
- No automated UI test infrastructure exists in the repo today; not adding one in this scope.

## Files touched

- `src-tauri/src/recorder.rs` — add `convert_to_gif` command + a small private helper to build the ffmpeg argv.
- `src-tauri/src/lib.rs` — register `recorder::convert_to_gif` in `invoke_handler!`.
- `src/routes/+page.svelte` — state, handler, UI, minor refactor of `openVideo`/`showInFolder` to take a path argument.
- `README.md` — add GIF conversion to the features list (per `CLAUDE.md` workflow rule: "update the README if anything user-facing changed").

## Open questions

None at design time. If conversion turns out to be too slow on long clips during testing, future work would be a progress indicator and/or cancellation — explicitly deferred.
