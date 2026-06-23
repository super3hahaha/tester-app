mod analysis;
mod auth;
mod chrome;
mod claude;
mod compare;
mod feedback;
mod manifest;
mod model_config;
mod reply;
mod reviews;
mod sheets;
mod skill_sync;
mod templates;
mod translate;

use analysis::AnalysisState;
use auth::AuthState;
use claude::ClaudeState;
use compare::CompareState;
use reply::ReplyState;
use translate::TranslateState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AuthState::new())
        .manage(ClaudeState::new())
        .manage(CompareState::new())
        .manage(ReplyState::new())
        .manage(TranslateState::new())
        .manage(AnalysisState::new())
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
            sheets::find_latest_export,
            sheets::upload_xlsx_to_drive,
            sheets::upload_xlsx_bytes_to_drive,
            claude::run_claude_task,
            claude::send_claude_input,
            claude::get_claude_status,
            claude::stop_claude,
            claude::get_claude_account,
            compare::export_sheet_html,
            compare::run_diff_skill,
            compare::open_in_chrome,
            manifest::write_generate_manifest,
            feedback::is_feedback_configured,
            feedback::send_feedback,
            feedback::retry_pending_feedback,
            skill_sync::check_skill_updates,
            skill_sync::sync_all_skills,
            skill_sync::sync_skill,
            skill_sync::get_skill_local_version,
            reviews::list_play_reviews,
            reviews::list_play_apps,
            reviews::reply_to_review,
            reviews::save_reviews_snapshot,
            reviews::load_reviews_snapshot,
            reply::run_reply_skill,
            reply::stop_reply,
            reply::generate_single_reply,
            reply::generate_mail_reply,
            templates::list_template_products,
            templates::create_template_product,
            templates::product_for_package,
            templates::list_templates,
            templates::add_template,
            templates::update_template,
            templates::delete_template,
            templates::delete_template_product,
            templates::get_package_map,
            templates::save_package_map,
            templates::import_templates_xlsx,
            templates::export_templates_xlsx,
            templates::set_template_translation,
            translate::translate_templates,
            translate::stop_translate,
            analysis::list_knowledge,
            analysis::read_knowledge,
            analysis::write_knowledge,
            analysis::generate_analysis,
            analysis::stop_analysis,
            model_config::get_model_config,
            model_config::save_model_config,
            chrome::list_chrome_profiles,
            chrome::open_url_in_chrome_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
