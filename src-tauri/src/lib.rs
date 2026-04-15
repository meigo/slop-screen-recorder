mod overlay;
mod recorder;

use recorder::RecorderState;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(RecorderState {
            process: Mutex::new(None),
            output_path: Mutex::new(None),
            ffmpeg_path: Mutex::new(None),
        })
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
