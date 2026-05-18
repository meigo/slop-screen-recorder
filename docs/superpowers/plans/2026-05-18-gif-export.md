# GIF Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-recording "Convert to GIF" action that post-processes the most recent MP4 into a clean, small GIF using a single ffmpeg invocation.

**Architecture:** New Tauri command `convert_to_gif(input_path)` in `src-tauri/src/recorder.rs` shells out to the bundled ffmpeg with a `mpdecimate` + `palettegen`/`paletteuse=diff_mode=rectangle` filter graph. Frontend (`src/routes/+page.svelte`) gets a new button in the existing output-actions row and a second info row for the resulting GIF.

**Tech Stack:** Rust (Tauri 2.10) backend, SvelteKit 5 + TypeScript frontend, ffmpeg sidecar, lucide-svelte icons.

**Spec:** `docs/superpowers/specs/2026-05-18-gif-export-design.md`

---

## File Structure

| File | Change | Responsibility |
| ---- | ------ | -------------- |
| `src-tauri/src/recorder.rs` | Modify | Add pure `build_gif_args` helper + `#[tauri::command] convert_to_gif`. Add `#[cfg(test)] mod tests` block. |
| `src-tauri/src/lib.rs` | Modify | Register `recorder::convert_to_gif` in `invoke_handler!`. |
| `src/routes/+page.svelte` | Modify | New `gifPath` / `convertingGif` state, `convertToGif` handler, GIF button, second info row, spin animation. Small refactor: `openVideo`/`showInFolder` become path-parameterised `openFile`/`revealFile`. |
| `README.md` | Modify | Add GIF conversion bullet to features. |

Everything stays inside existing files. No new modules.

---

### Task 1: Pure `build_gif_args` helper with unit test

Adds the argv-builder function and its test. Pure function, no I/O. This is the only piece of the backend that's worth unit-testing.

**Files:**
- Modify: `src-tauri/src/recorder.rs` (append at end of file, before any test module)
- Test: `src-tauri/src/recorder.rs` (new `#[cfg(test)] mod tests` block at end)

- [ ] **Step 1: Add the failing test**

At the very end of `src-tauri/src/recorder.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_gif_args_contains_paths_and_filter() {
        let args = build_gif_args("/tmp/in.mp4", "/tmp/out.gif");

        // Output is the last positional argument.
        assert_eq!(args.last().map(String::as_str), Some("/tmp/out.gif"));

        // Input follows `-i`.
        let i_pos = args.iter().position(|a| a == "-i").expect("missing -i");
        assert_eq!(args[i_pos + 1], "/tmp/in.mp4");

        // Filter graph contains the key pieces from the design.
        let f_pos = args
            .iter()
            .position(|a| a == "-filter_complex")
            .expect("missing -filter_complex");
        let filter = &args[f_pos + 1];
        assert!(filter.contains("fps=10"));
        assert!(filter.contains("mpdecimate"));
        assert!(filter.contains("palettegen=stats_mode=diff"));
        assert!(filter.contains("paletteuse=diff_mode=rectangle"));

        // Overwrite + loop forever.
        assert!(args.iter().any(|a| a == "-y"));
        let loop_pos = args.iter().position(|a| a == "-loop").expect("missing -loop");
        assert_eq!(args[loop_pos + 1], "0");
    }
}
```

- [ ] **Step 2: Run the test, verify it fails**

```sh
cd src-tauri && cargo test build_gif_args_contains_paths_and_filter
```

Expected: compile error — `build_gif_args` is undefined.

- [ ] **Step 3: Implement the helper**

In `src-tauri/src/recorder.rs`, just above the `#[cfg(test)]` block you just added, insert:

```rust
/// Build the ffmpeg argv for converting an MP4 to a GIF.
///
/// The filter graph:
///   - caps the rate at 10fps (longest delays come from `mpdecimate` skipping
///     near-identical frames, so static stretches collapse into single long-
///     delay GIF frames),
///   - downscales width to at most 720px (lanczos), height auto + even,
///   - generates a per-clip palette weighted toward changing pixels, and
///   - applies that palette with `diff_mode=rectangle` so each GIF frame only
///     re-encodes the bounding box of changed pixels.
fn build_gif_args(input: &str, output: &str) -> Vec<String> {
    let filter = "fps=10,scale='min(720,iw)':-2:flags=lanczos,mpdecimate,split[a][b];\
                  [a]palettegen=stats_mode=diff[p];\
                  [b][p]paletteuse=diff_mode=rectangle";
    vec![
        "-y".to_string(),
        "-i".to_string(),
        input.to_string(),
        "-filter_complex".to_string(),
        filter.to_string(),
        "-loop".to_string(),
        "0".to_string(),
        output.to_string(),
    ]
}
```

- [ ] **Step 4: Run the test, verify it passes**

```sh
cd src-tauri && cargo test build_gif_args_contains_paths_and_filter
```

Expected: `test build_gif_args_contains_paths_and_filter ... ok`.

- [ ] **Step 5: Commit**

```sh
git add src-tauri/src/recorder.rs
git commit -m "Add build_gif_args helper for MP4 → GIF conversion"
```

---

### Task 2: `convert_to_gif` Tauri command

Wires the helper into a registered Tauri command. No new unit test — this function is mostly I/O against a real ffmpeg binary; we'll verify it manually at the end.

**Files:**
- Modify: `src-tauri/src/recorder.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the command function**

In `src-tauri/src/recorder.rs`, right after the `get_default_output_dir` command (around line 616, before the `ChildExt` trait), insert:

```rust
#[tauri::command]
pub fn convert_to_gif(
    state: State<RecorderState>,
    input_path: String,
) -> Result<String, String> {
    let input_pb = std::path::PathBuf::from(&input_path);

    if !input_pb.exists() {
        return Err(format!("Input file does not exist: {}", input_path));
    }
    if input_pb.extension().and_then(|s| s.to_str()) != Some("mp4") {
        return Err("Input must be an .mp4 file".to_string());
    }

    let parent = input_pb
        .parent()
        .ok_or_else(|| "Input has no parent directory".to_string())?;
    let stem = input_pb
        .file_stem()
        .ok_or_else(|| "Input has no file name".to_string())?;
    let output_pb = parent.join(format!("{}.gif", stem.to_string_lossy()));
    let output_path = output_pb.to_string_lossy().to_string();

    let ffmpeg = find_ffmpeg(&state);
    let args = build_gif_args(&input_path, &output_path);

    log::info!("Converting to GIF with args: {:?}", args);

    let output = ffmpeg_command(&ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg failed: {}", stderr.trim()));
    }

    Ok(output_path)
}
```

- [ ] **Step 2: Register the command**

In `src-tauri/src/lib.rs`, edit the `invoke_handler!` macro list. Current state (lines 28-38):

```rust
.invoke_handler(tauri::generate_handler![
    recorder::check_ffmpeg,
    recorder::list_sources,
    recorder::list_audio_devices,
    recorder::start_recording,
    recorder::stop_recording,
    recorder::is_recording,
    recorder::get_default_output_dir,
    overlay::show_region_overlay,
    overlay::hide_region_overlay,
])
```

Add `recorder::convert_to_gif,` so it becomes:

```rust
.invoke_handler(tauri::generate_handler![
    recorder::check_ffmpeg,
    recorder::list_sources,
    recorder::list_audio_devices,
    recorder::start_recording,
    recorder::stop_recording,
    recorder::is_recording,
    recorder::get_default_output_dir,
    recorder::convert_to_gif,
    overlay::show_region_overlay,
    overlay::hide_region_overlay,
])
```

- [ ] **Step 3: Verify the Rust side compiles and lints cleanly**

```sh
npm run test:rust
```

Expected: cargo clippy reports no warnings; `cargo test` shows `build_gif_args_contains_paths_and_filter ... ok` and no failures.

- [ ] **Step 4: Commit**

```sh
git add src-tauri/src/recorder.rs src-tauri/src/lib.rs
git commit -m "Add convert_to_gif Tauri command"
```

---

### Task 3: Frontend state, handler, and refactor

Adds the reactive state and conversion handler, and refactors the two file-action helpers to take a path argument so the same code serves both MP4 and GIF.

**Files:**
- Modify: `src/routes/+page.svelte`

- [ ] **Step 1: Add the FileImage icon to the import**

In `src/routes/+page.svelte` lines 8-21, the icon import block currently ends with `ChevronRight,`. Replace the block:

```ts
import {
    MonitorDot,
    Sun,
    Moon,
    Folder,
    FolderOpen,
    Play,
    Square,
    Circle,
    Maximize2,
    Crosshair,
    RefreshCw,
    ChevronRight,
} from '@lucide/svelte';
```

with the same block plus `FileImage`:

```ts
import {
    MonitorDot,
    Sun,
    Moon,
    Folder,
    FolderOpen,
    Play,
    Square,
    Circle,
    Maximize2,
    Crosshair,
    RefreshCw,
    ChevronRight,
    FileImage,
} from '@lucide/svelte';
```

- [ ] **Step 2: Add the two new state variables**

Find the existing state block (lines 51-71). After the `let overlayVisible = $state(false);` line, append:

```ts
let gifPath = $state('');
let convertingGif = $state(false);
```

- [ ] **Step 3: Reset GIF state when a new recording starts**

In `startRecording` (around line 209), insert two lines as the very first statements of the function body, before `const config: RecordingConfig = { ... }`:

```ts
gifPath = '';
convertingGif = false;
```

- [ ] **Step 4: Refactor `openVideo` and `showInFolder` to take a path**

Find `openVideo` (around line 273-279) and `showInFolder` (around line 375-381). Replace both with:

```ts
async function openFile(path: string) {
    try {
        await openPath(path);
    } catch (e) {
        alert(`Failed to open: ${e}`);
    }
}

async function revealFile(path: string) {
    try {
        await revealItemInDir(path);
    } catch (e) {
        alert(`Failed to open folder: ${e}`);
    }
}
```

You can keep these together near the existing `pickOutputDir` function — the exact location doesn't matter as long as they're inside the `<script>` block.

- [ ] **Step 5: Add the `convertToGif` handler**

Anywhere convenient in the `<script>` block (e.g., right after the new `revealFile` function from Step 4), add:

```ts
async function convertToGif() {
    if (!outputPath || convertingGif) return;
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

- [ ] **Step 6: Verify the script still type-checks**

```sh
npm run check
```

Expected: no errors. (There will be warnings about `openVideo`/`showInFolder` being unreferenced from the template since we haven't updated the template yet — those resolve in Task 4. If `svelte-check` flags them as errors, that's fine for now; ignore until after Task 4.)

- [ ] **Step 7: Commit**

```sh
git add src/routes/+page.svelte
git commit -m "Add GIF conversion state and handler in record UI"
```

---

### Task 4: Frontend UI — button and second info row

Updates the template to wire up the new button and the GIF info row, and adds the spinner CSS. Also swaps the existing `openVideo`/`showInFolder` callsites over to the new helpers.

**Files:**
- Modify: `src/routes/+page.svelte`

- [ ] **Step 1: Replace the record-bar output block in the template**

Find the existing block (lines 565-578 in the original file):

```svelte
{#if outputPath && !recording}
    <div class="output-info">
        <code title={outputPath}>{outputPath.split(/[/\\]/).pop()}</code>
        <div class="output-actions">
            <button class="ghost" onclick={openVideo} aria-label="Open video">
                <Play size={13} strokeWidth={1.5} />
            </button>
            <button class="ghost" onclick={showInFolder} aria-label="Show in folder">
                <FolderOpen size={13} strokeWidth={1.5} />
            </button>
        </div>
    </div>
{/if}
```

Replace it with:

```svelte
{#if outputPath && !recording}
    <div class="output-info">
        <code title={outputPath}>{outputPath.split(/[/\\]/).pop()}</code>
        <div class="output-actions">
            <button
                class="ghost"
                onclick={() => openFile(outputPath)}
                aria-label="Open video"
            >
                <Play size={13} strokeWidth={1.5} />
            </button>
            <button
                class="ghost"
                onclick={() => revealFile(outputPath)}
                aria-label="Show in folder"
            >
                <FolderOpen size={13} strokeWidth={1.5} />
            </button>
            {#if !gifPath}
                <button
                    class="ghost"
                    onclick={convertToGif}
                    disabled={convertingGif}
                    aria-label="Convert to GIF"
                    title="Convert to GIF"
                >
                    {#if convertingGif}
                        <span class="spin"><RefreshCw size={13} strokeWidth={1.5} /></span>
                    {:else}
                        <FileImage size={13} strokeWidth={1.5} />
                    {/if}
                </button>
            {/if}
        </div>
    </div>

    {#if gifPath}
        <div class="output-info">
            <code title={gifPath}>{gifPath.split(/[/\\]/).pop()}</code>
            <div class="output-actions">
                <button
                    class="ghost"
                    onclick={() => openFile(gifPath)}
                    aria-label="Open GIF"
                >
                    <Play size={13} strokeWidth={1.5} />
                </button>
                <button
                    class="ghost"
                    onclick={() => revealFile(gifPath)}
                    aria-label="Show GIF in folder"
                >
                    <FolderOpen size={13} strokeWidth={1.5} />
                </button>
            </div>
        </div>
    {/if}
{/if}
```

- [ ] **Step 2: Add the spinner CSS**

Find the existing `@keyframes pulse` block (around lines 877-885 in the original file):

```css
@keyframes pulse {
    0%,
    100% {
        opacity: 1;
    }
    50% {
        opacity: 0.3;
    }
}
```

Immediately after it, append:

```css
.spin {
    display: inline-flex;
    animation: spin 1s linear infinite;
}

@keyframes spin {
    to {
        transform: rotate(360deg);
    }
}
```

- [ ] **Step 3: Verify the full test pipeline passes**

```sh
npm test
```

Expected: format, lint, svelte-check, clippy, and `cargo test` all clean. Specifically, `build_gif_args_contains_paths_and_filter ... ok` should appear in the cargo test output.

- [ ] **Step 4: Manual smoke test in dev mode**

```sh
npm run tauri dev
```

Then in the running app:

1. Pick a screen source, record a short clip (~5 seconds) that has at least a couple of seconds of no motion followed by some motion (e.g. wiggle a window). Stop.
2. The file name + Play + Show-in-folder + new GIF button should appear in the record bar. The GIF button shows the file-image icon.
3. Click the GIF button. The icon switches to a spinning refresh icon; the button is disabled.
4. After a few seconds (small clip = small wait), the spinner clears, the GIF button disappears, and a second info row appears below with the GIF's filename and its own Play + Show-in-folder buttons.
5. Play the GIF (opens in default image viewer). Confirm it loops and that static stretches of the source are visibly held (long delays), while motion plays back at up to 10fps.
6. Start a new recording, then stop it. Confirm the GIF row disappears and the GIF button reappears.
7. Try clicking the GIF button on a recording where you've deleted the underlying MP4 (delete from Explorer/Finder between steps). Confirm the alert surfaces the "Input file does not exist" error and the spinner state clears.

Note any issues. If anything in steps 1-6 fails, fix in this task; do not proceed to commit.

- [ ] **Step 5: Commit**

```sh
git add src/routes/+page.svelte
git commit -m "Add GIF convert button and result row to record UI"
```

---

### Task 5: README + final verification

Updates the features list per `CLAUDE.md`'s workflow rule, then runs the full pipeline once more as a final gate.

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add the GIF feature to the README**

In `README.md`, find the features list (lines 9-17). After the existing bullet:

```markdown
- **Audio capture** — optional microphone recording
```

insert a new bullet:

```markdown
- **GIF export** — one-click post-recording conversion to a size-conscious GIF (skips static frames, encodes only changed regions)
```

- [ ] **Step 2: Run the full pipeline**

```sh
npm test
```

Expected: all checks green.

- [ ] **Step 3: Commit**

```sh
git add README.md
git commit -m "Document GIF export in README features list"
```

---

## Self-Review

**Spec coverage:**

| Spec section | Implementing task(s) |
| ------------ | -------------------- |
| Convert button per-recording, no batch UI | Task 4 (only renders when `outputPath` set) |
| GIF row replaces button after success | Task 4 (`{#if !gifPath}` / `{#if gifPath}`) |
| Reset GIF state on new recording | Task 3, Step 3 |
| One ffmpeg invocation with smart filter | Task 1 + Task 2 |
| `fps=10`, max 720px width, lanczos | Task 1 filter string |
| `mpdecimate` + `paletteuse=diff_mode=rectangle` | Task 1 filter string |
| Validates input exists + .mp4 extension | Task 2, Step 1 |
| Overwrite existing output (`-y`) | Task 1 filter argv |
| Synchronous spawn via `.output()` | Task 2, Step 1 |
| Use `ffmpeg_command()` + `find_ffmpeg()` helpers | Task 2, Step 1 |
| Register command in `lib.rs` | Task 2, Step 2 |
| Spinner during conversion | Task 4, Steps 1 + 2 |
| Errors surfaced via `alert()` | Task 3, Step 5 |
| README updated (`CLAUDE.md` workflow rule) | Task 5 |
| `npm test` passes before completion | Task 4 Step 3 + Task 5 Step 2 |
| No progress / cancel (deferred) | (Non-goal — not in any task) |

All spec items have a home.

**Placeholder scan:** No "TBD", "TODO", "implement later", "add appropriate error handling" anywhere. Every code-changing step shows the actual code.

**Type / name consistency:**
- Tauri command name: `convert_to_gif` (snake_case in Rust), invoked as `'convert_to_gif'` with `{ inputPath: ... }` (Tauri 2 auto-converts) — used consistently in Task 2 Step 1, Task 2 Step 2, Task 3 Step 5.
- Helper: `build_gif_args` — defined Task 1 Step 3, asserted Task 1 Step 1, called Task 2 Step 1.
- Frontend state: `gifPath`, `convertingGif` — declared Task 3 Step 2, reset Task 3 Step 3, used Task 3 Step 5 and Task 4 Step 1.
- Refactored helpers: `openFile(path)`, `revealFile(path)` — declared Task 3 Step 4, used Task 3 Step 5 (via `convertToGif` indirectly — no, only used from template) and Task 4 Step 1.
- Old helpers `openVideo` / `showInFolder` are replaced in Task 3 Step 4 (removed) and Task 4 Step 1 (callsites swapped). The brief window between Step 4 of Task 3 and Step 1 of Task 4 leaves the template referring to removed identifiers — that's why `npm run check` is noted as potentially noisy at Task 3 Step 6 and the authoritative gate is `npm test` at Task 4 Step 3.

All consistent.
