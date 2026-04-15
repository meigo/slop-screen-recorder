use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

pub const OVERLAY_LABEL: &str = "region-overlay";

#[tauri::command]
pub fn show_region_overlay(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let w = width.max(40);
    let h = height.max(40);

    if let Some(win) = app.get_webview_window(OVERLAY_LABEL) {
        win.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
        win.set_size(PhysicalSize::new(w, h)).map_err(|e| e.to_string())?;
        win.show().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(&app, OVERLAY_LABEL, WebviewUrl::App("overlay".into()))
        .title("Region Overlay")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .shadow(false)
        .visible(false)
        .build()
        .map_err(|e| format!("Failed to create overlay: {}", e))?;

    window.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    window.set_size(PhysicalSize::new(w, h)).map_err(|e| e.to_string())?;
    window.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn hide_region_overlay(app: AppHandle) -> Result<(), String> {
    close_overlay(&app);
    Ok(())
}

pub fn close_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = win.close();
    }
}
