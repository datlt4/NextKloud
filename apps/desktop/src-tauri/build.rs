fn main() {
    // lệnh của app phải được cấp trong capabilities (desktop.json)
    let cmds = &["sync_auth", "sync_get", "sync_add", "sync_remove", "sync_pause", "sync_now"];
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(cmds)))
        .expect("tauri build");
}
