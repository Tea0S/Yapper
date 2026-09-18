# Yapper project conventions

After each completed batch of consumer-facing changes or bug fixes, bump the app version. Use a patch bump unless the user requests another version. Keep package.json, both root package-lock.json version fields, src-tauri/Cargo.toml, the Yapper entry in src-tauri/Cargo.lock, and src-tauri/tauri.conf.json synchronized. Update the Home release banner and its WHATS_NEW_VERSION key when preparing the corresponding release notes. Do not bump dependency versions or historical release references.
