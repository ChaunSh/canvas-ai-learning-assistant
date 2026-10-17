//! Canvas AI Learning Assistant — Tauri application library.
//!
//! The same Rust library backs the Windows desktop binary and the Android
//! package; the React frontend is shared verbatim between both.

pub mod ai;
pub mod commands;
pub mod domain;
pub mod documents;
pub mod error;
pub mod integrations;
pub mod paths;
pub mod planning;
pub mod retrieval;
pub mod security;
pub mod state;
pub mod storage;
pub mod time;

use tauri::Manager;

/// Initialise logging once, honouring `RUST_LOG`.
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    // rustls is built without the aws-lc-rs backend, so the provider must be
    // installed before the first HTTPS request.
    security::tls::install_crypto_provider();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = state::AppState::new(app.handle())?;
            app.manage(state);
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "Canvas AI Learning Assistant starting"
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // system
            commands::app_info,
            commands::settings_get,
            commands::settings_set,
            commands::settings_all,
            // courses
            commands::list_courses,
            commands::get_course,
            commands::create_course,
            commands::update_course,
            commands::delete_course,
            commands::list_course_modules,
            commands::create_course_module,
            commands::delete_course_module,
            commands::course_options,
            commands::get_course_detail,
            // coursework
            commands::list_assignments,
            commands::create_assignment,
            commands::update_assignment,
            commands::delete_assignment,
            commands::list_announcements,
            commands::mark_announcement_read,
            commands::list_grades,
            commands::list_discussions,
            commands::list_discussion_entries,
            commands::list_course_pages,
            commands::get_dashboard,
            // materials
            commands::import_materials,
            commands::list_materials,
            commands::get_material,
            commands::update_material,
            commands::delete_material,
            commands::material_bytes,
            commands::material_pages,
            commands::material_chunks,
            commands::index_material,
            commands::cancel_indexing,
            commands::search_materials,
            commands::list_summaries,
            commands::generate_summary,
            commands::delete_summary,
            commands::reindex_all,
            commands::library_stats,
            commands::indexing_status,
            // ai
            commands::list_providers,
            commands::save_provider,
            commands::delete_provider,
            commands::set_default_provider,
            commands::test_provider,
            commands::list_models,
            commands::ask_question,
            commands::cancel_generation,
            commands::list_chat_sessions,
            commands::get_chat_session,
            commands::delete_chat_session,
            commands::rename_chat_session,
            commands::update_chat_scope,
            commands::ai_usage,
            // planning
            commands::get_study_preferences,
            commands::update_study_preferences,
            commands::generate_study_plan,
            commands::replan_schedule,
            commands::list_study_plans,
            commands::delete_study_plan,
            commands::set_study_plan_status,
            commands::list_study_tasks,
            commands::create_study_task,
            commands::update_study_task,
            commands::complete_study_task,
            commands::delete_study_task,
            commands::upcoming_deadlines,
            commands::start_study_session,
            commands::finish_study_session,
            commands::list_study_sessions,
            commands::list_calendar_events,
            commands::create_calendar_event,
            commands::delete_calendar_event,
            commands::import_ics,
            commands::export_ics,
            commands::calendar_month,
            // canvas
            commands::get_canvas_connection,
            commands::open_canvas_login,
            commands::open_canvas_page,
            commands::disconnect_canvas,
            commands::check_canvas_connection,
            commands::get_canvas_capabilities,
            commands::sync_all,
            commands::sync_course,
            commands::get_sync_history,
            commands::sync_history_detailed,
            commands::get_sync_diagnostics,
            commands::export_sync_log,
            commands::canvas_integration_info,
            commands::list_course_files,
            commands::download_canvas_file,
            commands::download_course_files,
            // data
            commands::export_user_data,
            commands::import_user_data,
            commands::backup_database,
            commands::cache_info,
            commands::clear_cached_files,
            commands::delete_course_data,
            commands::reset_application_data,
            commands::storage_overview,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Canvas AI Learning Assistant");
}
