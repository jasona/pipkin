use gpui::App;
use gpui_platform::application;

mod adapters;
mod cache;
mod controller;
mod install;
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
                "usage: pipkin [--version] [--diagnose [--probe]] [--pi-dir <path>] [--pi-server-id <uuid>] [--data-dir <path>]\n       pipkin --demo <scenario> [--data-dir <path>] [--speed <factor>]"
            );
            std::process::exit(2);
        }
    };
    match options.action {
        controller::Action::Run => {}
        controller::Action::Version => {
            println!("pipkin {}", install::VERSION);
            return;
        }
        controller::Action::Diagnose { probe } => {
            let data_dir = platform::data_dir(options.data_dir.as_deref());
            let (text, bad) = install::diagnose(&data_dir, options.pi_repo.as_deref(), probe);
            print!("{text}");
            std::process::exit(if bad { 1 } else { 0 });
        }
    }
    let data_dir = platform::data_dir(options.data_dir.as_deref());
    application()
        .with_assets(pipkin_ui::assets::Assets)
        .run(|cx: &mut App| {
            let model = controller::start(cx, options);
            pipkin_ui::shell::open_main_window(cx, model, data_dir);
        });
}
