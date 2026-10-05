use gpui::App;
use gpui_platform::application;

mod adapters;
mod cache;
mod controller;
mod launch;
mod platform;
mod storage;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let options = match controller::Options::from_env_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("pipkin: {message}");
            eprintln!(
                "usage: pipkin [--pi-dir <path>] [--pi-server-id <uuid>] [--data-dir <path>]\n       pipkin --demo <scenario> [--data-dir <path>] [--speed <factor>]"
            );
            std::process::exit(2);
        }
    };
    application()
        .with_assets(pipkin_ui::assets::Assets)
        .run(|cx: &mut App| {
            let model = controller::start(cx, options);
            pipkin_ui::shell::open_main_window(cx, model);
        });
}
