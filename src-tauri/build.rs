fn main() {
    // createUpdaterArtifacts requires minisign key in env; PATH alone is not enough for tauri-build.
    // Load gitignored `.tauri/updater.key` so `cargo build` / rust-analyzer work without a shell wrapper.
    if std::env::var("TAURI_SIGNING_PRIVATE_KEY").is_err() {
        let key_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tauri/updater.key");
        if key_path.is_file() {
            if let Ok(contents) = std::fs::read_to_string(&key_path) {
                std::env::set_var("TAURI_SIGNING_PRIVATE_KEY", contents);
            }
        }
    }
    tauri_build::build()
}
