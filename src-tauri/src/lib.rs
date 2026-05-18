mod auth;
mod claude;
mod sheets;

use auth::AuthState;
use claude::ClaudeState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AuthState::new())
        .manage(ClaudeState::new())
        .invoke_handler(tauri::generate_handler![
            auth::check_auth,
            auth::start_login,
            auth::logout,
            sheets::list_drive_files,
            sheets::get_sheet_tabs,
            sheets::read_sheet,
            sheets::export_sheet_csv,
            sheets::export_slides_pptx,
            sheets::get_presentation_slides,
            sheets::get_cache_size,
            sheets::clear_cache,
            claude::run_claude_task,
            claude::send_claude_input,
            claude::get_claude_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
