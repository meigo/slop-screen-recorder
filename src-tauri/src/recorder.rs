use serde::{Deserialize, Serialize};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::State;

pub struct RecorderState {
    pub process: Mutex<Option<Child>>,
    pub output_path: Mutex<Option<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecordingSource {
    pub id: String,
    pub name: String,
    pub source_type: String, // "screen" or "window"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub source_id: String,
    pub output_dir: String,
    pub fps: u32,
    pub capture_audio: bool,
    pub audio_device: Option<String>,
}

#[cfg(target_os = "macos")]
fn find_ffmpeg() -> String {
    // Check common homebrew paths, then fall back to PATH
    for path in &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }
    "ffmpeg".to_string()
}

#[cfg(target_os = "windows")]
fn find_ffmpeg() -> String {
    "ffmpeg".to_string()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn find_ffmpeg() -> String {
    "ffmpeg".to_string()
}

#[tauri::command]
pub fn check_ffmpeg() -> Result<bool, String> {
    let ffmpeg = find_ffmpeg();
    match Command::new(&ffmpeg).arg("-version").output() {
        Ok(output) => Ok(output.status.success()),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
pub fn list_sources() -> Result<Vec<RecordingSource>, String> {
    let ffmpeg = find_ffmpeg();

    #[cfg(target_os = "macos")]
    {
        let output = Command::new(&ffmpeg)
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
        // On Windows with gdigrab, the main desktop is always available
        Ok(vec![RecordingSource {
            id: "desktop".to_string(),
            name: "Desktop".to_string(),
            source_type: "screen".to_string(),
        }])
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
pub fn list_audio_devices() -> Result<Vec<RecordingSource>, String> {
    let ffmpeg = find_ffmpeg();

    #[cfg(target_os = "macos")]
    {
        let output = Command::new(&ffmpeg)
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
        let output = Command::new(&ffmpeg)
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
    state: State<RecorderState>,
    config: RecordingConfig,
) -> Result<String, String> {
    let mut process = state.process.lock().map_err(|e| e.to_string())?;
    if process.is_some() {
        return Err("Already recording".to_string());
    }

    let ffmpeg = find_ffmpeg();
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
            "-i".into(), config.source_id.clone(),
        ]);

        if config.capture_audio {
            if let Some(ref audio_device) = config.audio_device {
                args.extend_from_slice(&[
                    "-f".into(), "dshow".into(),
                    "-i".into(), format!("audio={}", audio_device),
                ]);
            }
        }

        args.extend_from_slice(&[
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
            "-i".into(), config.source_id.clone(),
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

    let child = Command::new(&ffmpeg)
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
                Ok(Some(_status)) => {}
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
