use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::State;

#[cfg(target_os = "windows")]
use std::ffi::OsString;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStringExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub struct RecorderState {
    pub process: Mutex<Option<Child>>,
    pub output_path: Mutex<Option<String>>,
    pub ffmpeg_path: Mutex<Option<PathBuf>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecordingSource {
    pub id: String,
    pub name: String,
    pub source_type: String, // "screen" or "window"
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub source_id: String,
    pub output_dir: String,
    pub fps: u32,
    pub capture_audio: bool,
    pub audio_device: Option<String>,
    pub region: Option<Region>,
}

/// Verify the recording output directory exists and is writable. Returns an
/// actionable error if not — on Windows this is often Controlled Folder Access
/// blocking the write, which would otherwise surface as a misleading
/// "No such file or directory" from ffmpeg.
fn check_output_writable(output_dir: &str) -> Result<(), String> {
    let dir = std::path::Path::new(output_dir);
    if !dir.is_dir() {
        return Err(format!("Output directory does not exist: {}", output_dir));
    }
    let probe = dir.join(".slop_write_probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => {
            let extra = if cfg!(target_os = "windows") {
                "\n\nOn Windows this is often Controlled Folder Access blocking the app. \
                 Either pick a different output directory, or allow this app in \
                 Windows Security \u{2192} Virus & threat protection \u{2192} \
                 Ransomware protection \u{2192} Controlled folder access."
            } else {
                ""
            };
            Err(format!("Cannot write to {}: {}{}", output_dir, e, extra))
        }
    }
}

/// Tail ffmpeg's stderr to the last `max_lines` non-empty lines.
/// ffmpeg's verbose banner is at the start; the actual error is at the end,
/// so the tail is what's worth surfacing to the user.
fn tail_stderr(stderr: &str, max_lines: usize) -> String {
    let cleaned = stderr.replace('\r', "\n");
    let lines: Vec<&str> = cleaned.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

/// Create a Command that won't spawn a visible console window on Windows.
fn ffmpeg_command(path: &PathBuf) -> Command {
    #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
    let mut cmd = Command::new(path);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Resolve the ffmpeg binary path. Checks the cached path first, then
/// tries the bundled sidecar (next to the app executable), then falls
/// back to the system PATH.
fn find_ffmpeg(state: &State<RecorderState>) -> PathBuf {
    // Return cached path if we have one
    if let Ok(guard) = state.ffmpeg_path.lock() {
        if let Some(ref path) = *guard {
            return path.clone();
        }
    }

    let resolved = resolve_ffmpeg_path();

    // Cache it
    if let Ok(mut guard) = state.ffmpeg_path.lock() {
        *guard = Some(resolved.clone());
    }

    resolved
}

fn resolve_ffmpeg_path() -> PathBuf {
    // Try bundled sidecar: next to the current executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            #[cfg(target_os = "windows")]
            let sidecar = exe_dir.join("ffmpeg.exe");
            #[cfg(not(target_os = "windows"))]
            let sidecar = exe_dir.join("ffmpeg");

            if sidecar.exists() && sidecar.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                log::info!("Using bundled ffmpeg: {}", sidecar.display());
                return sidecar;
            }
        }
    }

    // Fall back to system PATH
    #[cfg(target_os = "macos")]
    for path in &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        if std::path::Path::new(path).exists() {
            return PathBuf::from(path);
        }
    }

    // On Windows, resolve the full path via `where` to avoid picking up
    // a 0-byte sidecar placeholder from the working directory.
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = Command::new("where").arg("ffmpeg").creation_flags(CREATE_NO_WINDOW).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let path = std::path::Path::new(line.trim());
                    if path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                        log::info!("Using system ffmpeg: {}", path.display());
                        return PathBuf::from(path);
                    }
                }
            }
        }
    }

    log::info!("Using system ffmpeg from PATH");
    PathBuf::from("ffmpeg")
}

#[tauri::command]
pub fn check_ffmpeg(state: State<RecorderState>) -> Result<bool, String> {
    let ffmpeg = find_ffmpeg(&state);
    log::info!("check_ffmpeg: trying {:?}", ffmpeg);
    match ffmpeg_command(&ffmpeg).arg("-version").output() {
        Ok(output) => {
            log::info!("check_ffmpeg: status={}", output.status.success());
            Ok(output.status.success())
        }
        Err(e) => {
            log::error!("check_ffmpeg: error: {}", e);
            Ok(false)
        }
    }
}

#[tauri::command]
#[allow(unused_variables)]
pub fn list_sources(state: State<RecorderState>) -> Result<Vec<RecordingSource>, String> {
    #[cfg(target_os = "macos")]
    {
        let ffmpeg = find_ffmpeg(&state);
        let output = ffmpeg_command(&ffmpeg)
            .args(["-f", "avfoundation", "-list_devices", "true", "-i", ""])
            .output()
            .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut sources = Vec::new();

        let mut in_video = false;
        for line in stderr.lines() {
            if line.contains("AVFoundation video devices:") {
                in_video = true;
                continue;
            }
            if line.contains("AVFoundation audio devices:") {
                break;
            }
            if in_video {
                // Parse lines like: [AVFoundation indev @ 0x...] [0] FaceTime HD Camera
                // or: [AVFoundation indev @ 0x...] [2] Capture screen 0
                if let Some(bracket_start) = line.find("] [") {
                    let rest = &line[bracket_start + 3..];
                    if let Some(bracket_end) = rest.find(']') {
                        let id = rest[..bracket_end].to_string();
                        let name = rest[bracket_end + 2..].to_string();
                        let source_type = if name.to_lowercase().contains("screen") || name.to_lowercase().contains("display") {
                            "screen"
                        } else {
                            "window"
                        };
                        sources.push(RecordingSource {
                            id,
                            name,
                            source_type: source_type.to_string(),
                        });
                    }
                }
            }
        }

        Ok(sources)
    }

    #[cfg(target_os = "windows")]
    {
        let mut sources = vec![RecordingSource {
            id: "desktop".to_string(),
            name: "Desktop (Full Screen)".to_string(),
            source_type: "screen".to_string(),
        }];

        sources.extend(list_windows());

        Ok(sources)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(vec![RecordingSource {
            id: ":0.0".to_string(),
            name: "Display :0".to_string(),
            source_type: "screen".to_string(),
        }])
    }
}

#[tauri::command]
pub fn list_audio_devices(state: State<RecorderState>) -> Result<Vec<RecordingSource>, String> {
    let ffmpeg = find_ffmpeg(&state);

    #[cfg(target_os = "macos")]
    {
        let output = ffmpeg_command(&ffmpeg)
            .args(["-f", "avfoundation", "-list_devices", "true", "-i", ""])
            .output()
            .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut devices = Vec::new();
        let mut in_audio = false;

        for line in stderr.lines() {
            if line.contains("AVFoundation audio devices:") {
                in_audio = true;
                continue;
            }
            if in_audio {
                if let Some(bracket_start) = line.find("] [") {
                    let rest = &line[bracket_start + 3..];
                    if let Some(bracket_end) = rest.find(']') {
                        let id = rest[..bracket_end].to_string();
                        let name = rest[bracket_end + 2..].to_string();
                        devices.push(RecordingSource {
                            id,
                            name,
                            source_type: "audio".to_string(),
                        });
                    }
                }
            }
        }

        Ok(devices)
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, list dshow audio devices
        let output = ffmpeg_command(&ffmpeg)
            .args(["-f", "dshow", "-list_devices", "true", "-i", "dummy"])
            .output()
            .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut devices = Vec::new();
        let mut in_audio = false;

        for line in stderr.lines() {
            if line.contains("DirectShow audio devices") {
                in_audio = true;
                continue;
            }
            if in_audio && line.contains("\"") {
                // Parse lines like: [dshow @ ...] "Microphone (Realtek Audio)"
                if let Some(start) = line.find('"') {
                    if let Some(end) = line[start + 1..].find('"') {
                        let name = line[start + 1..start + 1 + end].to_string();
                        devices.push(RecordingSource {
                            id: name.clone(),
                            name,
                            source_type: "audio".to_string(),
                        });
                    }
                }
            }
        }

        Ok(devices)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(vec![RecordingSource {
            id: "default".to_string(),
            name: "Default Audio".to_string(),
            source_type: "audio".to_string(),
        }])
    }
}

#[cfg(target_os = "windows")]
fn list_windows() -> Vec<RecordingSource> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindowVisible,
    };

    unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows = &mut *(lparam as *mut Vec<RecordingSource>);

        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return 1; // skip hidden and minimized windows
        }

        let len = GetWindowTextLengthW(hwnd);
        if len == 0 {
            return 1;
        }

        let mut buf = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if copied > 0 {
            let title = OsString::from_wide(&buf[..copied as usize])
                .to_string_lossy()
                .to_string();

            // Skip empty titles and known system windows
            if !title.is_empty()
                && title != "Program Manager"
                && title != "Windows Input Experience"
                && title != "MSCTFIME UI"
                && title != "Default IME"
            {
                windows.push(RecordingSource {
                    id: format!("hwnd:{}", hwnd as isize),
                    name: title,
                    source_type: "window".to_string(),
                });
            }
        }

        1 // continue enumeration
    }

    let mut windows: Vec<RecordingSource> = Vec::new();
    unsafe {
        EnumWindows(Some(enum_callback), &mut windows as *mut Vec<RecordingSource> as LPARAM);
    }

    // Deduplicate by title (keep first occurrence)
    let mut seen = std::collections::HashSet::new();
    windows.retain(|w| seen.insert(w.name.clone()));

    windows
}

/// Look up a window's screen rect by HWND for region-based capture.
#[cfg(target_os = "windows")]
fn get_window_rect_by_hwnd(hwnd_val: isize) -> Result<(i32, i32, i32, i32), String> {
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows_sys::Win32::Foundation::RECT;

    let hwnd = hwnd_val as windows_sys::Win32::Foundation::HWND;
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };

    if hr != 0 {
        return Err(format!("DwmGetWindowAttribute failed: 0x{:08x}", hr));
    }

    let x = rect.left;
    let y = rect.top;
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;

    if w <= 0 || h <= 0 {
        return Err("Window has zero size".to_string());
    }

    Ok((x, y, w, h))
}

fn generate_output_path(output_dir: &str) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let filename = format!("recording_{}.mp4", timestamp);
    std::path::Path::new(output_dir)
        .join(filename)
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
pub fn start_recording(
    app: tauri::AppHandle,
    state: State<RecorderState>,
    config: RecordingConfig,
) -> Result<String, String> {
    let mut process = state.process.lock().map_err(|e| e.to_string())?;
    if process.is_some() {
        return Err("Already recording".to_string());
    }

    // Pre-flight: confirm we can actually write to the output directory before
    // spawning ffmpeg. ffmpeg's "No such file or directory" for write failures
    // is misleading, and a failed recording wastes the user's time.
    check_output_writable(&config.output_dir)?;

    // Close the layout overlay so its outline doesn't end up in the recording.
    crate::overlay::close_overlay(&app);

    let ffmpeg = find_ffmpeg(&state);
    let output_path = generate_output_path(&config.output_dir);

    let mut args: Vec<String> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        args.extend_from_slice(&[
            "-f".into(), "avfoundation".into(),
            "-capture_cursor".into(), "1".into(),
            "-framerate".into(), config.fps.to_string(),
        ]);

        if config.capture_audio {
            let audio_id = config.audio_device.as_deref().unwrap_or("0");
            args.extend_from_slice(&[
                "-i".into(), format!("{}:{}", config.source_id, audio_id),
            ]);
        } else {
            args.extend_from_slice(&[
                "-i".into(), format!("{}:none", config.source_id),
            ]);
        }

        // Crop to region if requested (avfoundation has no native region flag)
        if let Some(r) = config.region {
            let w = r.width & !1;
            let h = r.height & !1;
            args.extend_from_slice(&[
                "-vf".into(), format!("crop={}:{}:{}:{}", w, h, r.x, r.y),
            ]);
        }

        // Video encoding — try hardware acceleration first
        args.extend_from_slice(&[
            "-c:v".into(), "h264_videotoolbox".into(),
            "-b:v".into(), "5000k".into(),
            "-pix_fmt".into(), "yuv420p".into(),
        ]);

        if config.capture_audio {
            args.extend_from_slice(&[
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "128k".into(),
            ]);
        }
    }

    #[cfg(target_os = "windows")]
    {
        args.extend_from_slice(&[
            "-f".into(), "gdigrab".into(),
            "-framerate".into(), config.fps.to_string(),
        ]);

        // Compute capture rect: window handle -> window bounds; region -> user rect; else full desktop
        let is_window_capture = config.source_id.starts_with("hwnd:");
        if is_window_capture && config.region.is_some() {
            return Err("Region capture is not supported with a window source".to_string());
        }
        let rect = if is_window_capture {
            let hwnd: isize = config.source_id[5..]
                .parse()
                .map_err(|_| "Invalid window handle".to_string())?;
            Some(get_window_rect_by_hwnd(hwnd)?)
        } else {
            config.region.map(|r| (r.x, r.y, r.width as i32, r.height as i32))
        };

        if let Some((x, y, w, h)) = rect {
            let w = w & !1;
            let h = h & !1;
            args.extend_from_slice(&[
                "-offset_x".into(), x.to_string(),
                "-offset_y".into(), y.to_string(),
                "-video_size".into(), format!("{}x{}", w, h),
                "-i".into(), "desktop".into(),
            ]);
        } else {
            args.extend_from_slice(&[
                "-i".into(), "desktop".into(),
            ]);
        }

        if config.capture_audio {
            if let Some(ref audio_device) = config.audio_device {
                args.extend_from_slice(&[
                    "-f".into(), "dshow".into(),
                    "-i".into(), format!("audio={}", audio_device),
                ]);
            }
        }

        // For window capture, use vf pad to ensure even dimensions;
        // for full screen, use crop filter as fallback
        args.extend_from_slice(&[
            "-vf".into(), "pad=ceil(iw/2)*2:ceil(ih/2)*2".into(),
            "-c:v".into(), "libx264".into(),
            "-preset".into(), "ultrafast".into(),
            "-pix_fmt".into(), "yuv420p".into(),
            "-b:v".into(), "5000k".into(),
        ]);

        if config.capture_audio {
            args.extend_from_slice(&[
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "128k".into(),
            ]);
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        args.extend_from_slice(&[
            "-f".into(), "x11grab".into(),
            "-framerate".into(), config.fps.to_string(),
        ]);

        let input = if let Some(r) = config.region {
            let w = r.width & !1;
            let h = r.height & !1;
            args.extend_from_slice(&[
                "-video_size".into(), format!("{}x{}", w, h),
            ]);
            format!("{}+{},{}", config.source_id, r.x, r.y)
        } else {
            config.source_id.clone()
        };

        args.extend_from_slice(&[
            "-i".into(), input,
            "-c:v".into(), "libx264".into(),
            "-preset".into(), "ultrafast".into(),
            "-pix_fmt".into(), "yuv420p".into(),
            "-b:v".into(), "5000k".into(),
        ]);
    }

    // Overwrite output without asking
    args.push("-y".into());
    args.push(output_path.clone());

    log::info!("Starting ffmpeg with args: {:?}", args);

    let child = ffmpeg_command(&ffmpeg)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start ffmpeg: {}", e))?;

    *process = Some(child);
    *state.output_path.lock().map_err(|e| e.to_string())? = Some(output_path.clone());

    Ok(output_path)
}

#[tauri::command]
pub fn stop_recording(state: State<RecorderState>) -> Result<String, String> {
    let mut process = state.process.lock().map_err(|e| e.to_string())?;

    match process.take() {
        Some(mut child) => {
            // Send 'q' to ffmpeg's stdin for graceful shutdown
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(b"q");
                let _ = stdin.flush();
            }

            // Wait for ffmpeg to finish writing the file
            match child.wait_timeout(std::time::Duration::from_secs(10)) {
                Ok(Some(status)) => {
                    if !status.success() {
                        let mut stderr = String::new();
                        if let Some(mut s) = child.stderr.take() {
                            use std::io::Read;
                            let _ = s.read_to_string(&mut stderr);
                        }
                        return Err(format!("ffmpeg failed: {}", tail_stderr(&stderr, 6)));
                    }
                }
                Ok(None) => {
                    // Timeout — force kill
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(e) => {
                    let _ = child.kill();
                    return Err(format!("Error waiting for ffmpeg: {}", e));
                }
            }

            let output_path = state.output_path.lock().map_err(|e| e.to_string())?;
            Ok(output_path.clone().unwrap_or_default())
        }
        None => Err("Not recording".to_string()),
    }
}

#[tauri::command]
pub fn is_recording(state: State<RecorderState>) -> bool {
    state
        .process
        .lock()
        .map(|p| p.is_some())
        .unwrap_or(false)
}

#[tauri::command]
pub fn get_default_output_dir() -> String {
    dirs::video_dir()
        .or_else(dirs::desktop_dir)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
        .to_string_lossy()
        .to_string()
}

/// Convert an MP4 recording to a GIF. Blocks until ffmpeg finishes;
/// returns the output GIF path on success, or a tailed ffmpeg stderr
/// fragment on failure.
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
        return Err(format!("ffmpeg failed: {}", tail_stderr(&stderr, 6)));
    }

    Ok(output_path)
}

// Trait extension for wait_timeout on Child
trait ChildExt {
    fn wait_timeout(&mut self, dur: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl ChildExt for Child {
    fn wait_timeout(&mut self, dur: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        let start = std::time::Instant::now();
        loop {
            match self.try_wait()? {
                Some(status) => return Ok(Some(status)),
                None => {
                    if start.elapsed() >= dur {
                        return Ok(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    }
}

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
        assert!(filter.contains("scale="));
        assert!(filter.contains("mpdecimate"));
        assert!(filter.contains("palettegen=stats_mode=diff"));
        assert!(filter.contains("paletteuse=diff_mode=rectangle"));

        // Overwrite + loop forever.
        assert!(args.iter().any(|a| a == "-y"));
        let loop_pos = args.iter().position(|a| a == "-loop").expect("missing -loop");
        assert_eq!(args[loop_pos + 1], "0");
    }

    #[test]
    fn tail_stderr_returns_last_n_non_empty_lines() {
        let input = "ffmpeg version 7.1.1\n  built with...\n  configuration: ...\n\n\
                     [gdigrab @ 0x1] capturing\n\
                     [out#0/mp4 @ 0x2] Error opening output: No such file or directory\n\
                     Error opening output file\n\
                     Error opening output files: No such file or directory\n";
        let tail = tail_stderr(input, 3);
        let lines: Vec<&str> = tail.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(tail.contains("Error opening output files"));
        assert!(!tail.contains("ffmpeg version"));
    }

    #[test]
    fn tail_stderr_treats_carriage_returns_as_newlines() {
        // ffmpeg's progress lines overwrite with \r; treat each as a separate line
        // so they don't crowd out the final error.
        let input = "frame=1 fps=0\rframe=2 fps=30\rframe=3 fps=30\rError: something\n";
        let tail = tail_stderr(input, 2);
        assert!(tail.contains("Error: something"));
        assert!(!tail.contains("frame=1"));
    }

    #[test]
    fn tail_stderr_handles_input_shorter_than_max() {
        let input = "only line\n";
        let tail = tail_stderr(input, 6);
        assert_eq!(tail, "only line");
    }

    #[test]
    fn check_output_writable_rejects_missing_directory() {
        let result = check_output_writable("Z:\\definitely\\does\\not\\exist\\anywhere");
        let err = result.expect_err("expected an error for a missing directory");
        assert!(err.contains("does not exist"), "got: {}", err);
    }

    #[test]
    fn check_output_writable_accepts_a_writable_temp_dir() {
        let tmp = std::env::temp_dir();
        check_output_writable(&tmp.to_string_lossy()).expect("temp dir should be writable");
    }
}
