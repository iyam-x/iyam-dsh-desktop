use tauri::generate_handler;
mod installer;
mod process;
mod updater;
mod window;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .invoke_handler(generate_handler![
            installer::get_install_status,
            installer::check_and_install,
            process::start_dsh,
            process::stop_dsh,
            updater::check_for_update,
            window::show_system_menu,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
