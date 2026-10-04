use gpui::App;
use gpui_platform::application;

mod adapters;
mod controller;
mod platform;
mod storage;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let options = match controller::Options::from_env_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("pi-desktop: {message}");
            eprintln!(
                "usage: desktop-app [--demo <scenario>] [--data-dir <path>] [--speed <factor>]"
            );
            std::process::exit(2);
        }
    };
    application()
        .with_assets(desktop_ui::assets::Assets)
        .run(|cx: &mut App| {
            let model = controller::start(cx, options);
            desktop_ui::shell::open_main_window(cx, model);
        });
}
