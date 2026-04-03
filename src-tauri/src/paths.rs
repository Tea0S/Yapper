use std::path::PathBuf;
use tauri::Manager;

pub fn db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("yapper.db"))
}

pub fn model_cache_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Resolves `sidecar/server.py` for dev, installed layout next to the exe, or Tauri bundled resources.
pub fn sidecar_script_path(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("YAPPER_SIDECAR") {
        return PathBuf::from(p);
    }
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("sidecar")
            .join("server.py");
    }
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let next_to_exe = exe.join("sidecar").join("server.py");
        if next_to_exe.is_file() {
            return next_to_exe;
        }
    }
    if let Ok(res) = app.path().resource_dir() {
        let bundled = res.join("sidecar").join("server.py");
        if bundled.is_file() {
            return bundled;
        }
    }
    std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .map(|p| p.join("sidecar").join("server.py"))
        .unwrap_or_else(|| PathBuf::from("sidecar/server.py"))
}
