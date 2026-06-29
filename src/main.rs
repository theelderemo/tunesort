#![recursion_limit = "256"]

mod config;
mod library;
mod metadata;
mod player;
mod ui;

use adw::prelude::*;
use config::Config;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

const APP_ID: &str = "io.github.theelderemo.tunesort";

fn main() {
    let mut start_library: Option<String> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--browser" => {}
            "-h" | "--help" => {
                println!(
                    "tunesort - dark keyboard-driven music sorter\n\n\
                     USAGE:\n    tunesort [LIBRARY]\n\n\
                     ARGS:\n    <LIBRARY>    optional path to load on start\n"
                );
                return;
            }
            other if other.starts_with("--port") => {}
            other if other.starts_with('-') => {}
            other => start_library = Some(other.to_string()),
        }
    }

    if let Some(ref lib) = start_library {
        if Path::new(lib).is_dir() {
            let mut cfg = Config::load();
            let abs = std::fs::canonicalize(lib)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| lib.clone());
            cfg.set_library_path(&abs);
        }
    }

    let app = adw::Application::builder().application_id(APP_ID).build();

    let started: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    app.connect_startup(|_| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    });
    app.connect_activate(move |app| {
        if *started.borrow() {
            return;
        }
        *started.borrow_mut() = true;
        ui::build(app);
    });

    let empty: Vec<String> = vec![std::env::args().next().unwrap_or_default()];
    app.run_with_args(&empty);
}
