use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::fs;
use tauri::{AppHandle, Manager};

const KEEP_LAST: usize = 5;

/// Saves a captured speech clip (base64 WAV) into `<app data>/debug-audio`
/// and keeps only the newest `KEEP_LAST` files. Returns the saved path.
#[tauri::command]
pub fn save_debug_audio(app: AppHandle, audio_base64: String) -> Result<String, String> {
    let bytes = STANDARD
        .decode(audio_base64.as_bytes())
        .map_err(|e| format!("Invalid audio data: {e}"))?;

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("debug-audio");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("clip-{stamp}.wav"));
    fs::write(&path, bytes).map_err(|e| e.to_string())?;

    // Names sort chronologically because the stamp has a fixed width for centuries.
    let mut clips: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "wav"))
        .collect();
    clips.sort();
    while clips.len() > KEEP_LAST {
        let _ = fs::remove_file(clips.remove(0));
    }

    Ok(path.to_string_lossy().into_owned())
}
