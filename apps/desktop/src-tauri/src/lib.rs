#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // iOS không có stderr: ghi lý do panic ra Documents/nk-panic.txt để đọc lại khi app crash
    #[cfg(mobile)]
    std::panic::set_hook(Box::new(|i| {
        if let Ok(h) = std::env::var("HOME") {
            let _ = std::fs::write(format!("{h}/Documents/nk-panic.txt"), i.to_string());
        }
    }));
    let b = tauri::Builder::default();
    #[cfg(desktop)]
    let b = b.plugin(tauri_plugin_updater::Builder::new().build()).plugin(tauri_plugin_process::init());
    b.run(tauri::generate_context!()).expect("error while running NextKloud");
}
