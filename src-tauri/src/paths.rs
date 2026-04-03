use std::path::PathBuf;
use tauri::path::BaseDirectory;
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

/// Embeddable interpreter from `src-tauri/resources/python-runtime/` (bundled as `$RESOURCE/resources/...`).
pub fn bundled_python_exe(app: &tauri::AppHandle) -> Option<PathBuf> {
    let candidates = [
        // Matches `bundle.resources` entry `resources/**/*`
        app
            .path()
            .resolve("resources/python-runtime/python.exe", BaseDirectory::Resource)
            .ok(),
        // Legacy / mistaken join (keep if an older layout existed)
        app
            .path()
            .resource_dir()
            .ok()
            .map(|r| r.join("python-runtime").join("python.exe")),
    ];
    for p in candidates.into_iter().flatten() {
        if p.is_file() {
            return Some(p);
        }
    }
    None
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
    // Tauri maps `../` in bundle paths to `_up_/…` under resource_dir — use the same rules as the bundler.
    if let Ok(p) = app
        .path()
        .resolve("../sidecar/server.py", BaseDirectory::Resource)
    {
        if p.is_file() {
            return p;
        }
    }
    std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .map(|p| p.join("sidecar").join("server.py"))
        .unwrap_or_else(|| PathBuf::from("sidecar/server.py"))
}

/// Resolves `yapper-node/main.py` for dev, next to the exe, or bundled under resources.
pub fn yapper_node_main_path(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("YAPPER_NODE") {
        return PathBuf::from(p);
    }
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("yapper-node")
            .join("main.py");
    }
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let next_to_exe = exe.join("yapper-node").join("main.py");
        if next_to_exe.is_file() {
            return next_to_exe;
        }
    }
    if let Ok(p) = app
        .path()
        .resolve("../yapper-node/main.py", BaseDirectory::Resource)
    {
        if p.is_file() {
            return p;
        }
    }
    std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .map(|p| p.join("yapper-node").join("main.py"))
        .unwrap_or_else(|| PathBuf::from("yapper-node/main.py"))
}
