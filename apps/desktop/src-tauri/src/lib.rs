#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let b = tauri::Builder::default();
    // updater/process chỉ hỗ trợ desktop; đăng ký trên iOS làm app abort lúc mở
    #[cfg(desktop)]
    let b = b.plugin(tauri_plugin_updater::Builder::new().build()).plugin(tauri_plugin_process::init());
    b.run(tauri::generate_context!()).expect("error while running NextKloud");
}
