// SPDX-License-Identifier: GPL-3.0-only
//! Pace Beads user interface. The recorder keeps counting when it is closed.

mod chart;
mod client;
mod format;
mod preferences;
mod transfer;
mod window;

use adw::prelude::*;
use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};

pub const APP_ID: &str = "io.github.x_vlad_x.PaceBeads";

fn main() -> gtk::glib::ExitCode {
    setlocale(LocaleCategory::LcAll, "");
    let _ = bindtextdomain("pace-beads", pace_core::ipc::locale_directory());
    let _ = bind_textdomain_codeset("pace-beads", "UTF-8");
    let _ = textdomain("pace-beads");

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(|application| {
        if let Some(window) = application.active_window() {
            window.present();
            return;
        }
        window::build(application);
    });
    application.run()
}
