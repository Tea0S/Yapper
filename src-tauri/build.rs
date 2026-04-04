fn main() {
    // Updater signing runs in the Tauri CLI bundler (separate process from this build script),
    // so setting TAURI_SIGNING_PRIVATE_KEY here does not help `npm run tauri build`.
    // Default config has createUpdaterArtifacts false. For signed release artifacts, use
    // scripts/pack-with-updater-signing.* (loads .tauri/updater.key + merges tauri.updater-release.conf.json).
    tauri_build::build()
}
