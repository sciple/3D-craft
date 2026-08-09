pub mod commands;
pub mod geometry;
pub mod io;
pub mod scene;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_document,
            commands::undo,
            commands::redo,
            commands::draw_rectangle,
            commands::draw_circle,
            commands::draw_arc,
            commands::draw_ngon,
            commands::draw_polygon,
            commands::push_pull_face,
            commands::push_pull_faces,
            commands::inset_face,
            commands::erase_face,
            commands::move_faces,
            commands::drop_to_plate,
            commands::rotate_faces,
            commands::scale_faces,
            commands::duplicate_faces,
            commands::array_faces,
            commands::mirror_faces,
            commands::group_faces,
            commands::ungroup,
            commands::select_group,
            commands::select_faces,
            commands::add_guide,
            commands::clear_guides,
            commands::save_project,
            commands::load_project,
            commands::export_stl,
            commands::arrange_for_print,
            commands::check_model,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
