// SPDX-License-Identifier: GPL-3.0-only
//! Manual export and previewed, atomic import.

use crate::format;
use crate::window::Shared;
use adw::prelude::*;
use gettextrs::gettext;
use gtk::gio;
use pace_core::backup;
use pace_core::csv::{self, History};
use pace_core::store::read_stepsy_database;
use std::path::Path;

fn toast(shared: &Shared, text: &str) {
    shared.toasts.add_toast(adw::Toast::new(text));
}

pub fn export(shared: &Shared) {
    let dialog = gtk::FileDialog::builder()
        .title(gettext("Export History"))
        .initial_name(backup::file_name(chrono::Local::now()))
        .build();
    let shared = shared.clone();
    dialog.save(Some(&shared.window.clone()), gio::Cancellable::NONE, move |file| {
        let Some(path) = file.ok().and_then(|file| file.path()) else {
            return;
        };
        let written = shared.store.borrow().history().map_err(|error| error.to_string()).and_then(|history| {
            let directory = path.parent().unwrap_or(Path::new("."));
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("history.csv");
            backup::write_copy(directory, name, &history).map_err(|error| error.to_string())
        });
        match written {
            Ok(_) => toast(&shared, &gettext("History exported")),
            Err(error) => toast(&shared, &format::fill(&gettext("Export failed: {}"), &[&error])),
        }
    });
}

fn read_history(path: &Path) -> Result<History, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.starts_with(b"SQLite format 3\0") {
        return read_stepsy_database(path).map_err(|error| error.to_string());
    }
    let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    csv::parse(&text).map_err(|error| error.to_string())
}

fn confirm_import(shared: &Shared, history: History) {
    let (Some(first), Some(last)) = (history.keys().next(), history.keys().next_back()) else {
        toast(shared, &gettext("The file contains no history"));
        return;
    };
    let settings = shared.settings();
    let total: u64 = history.values().sum();
    let body = format::fill(
        &gettext("{} from {} to {}, {}. This replaces the current history."),
        &[
            &format::day_count(history.len() as u64),
            &format::date(&settings, *first),
            &format::date(&settings, *last),
            &format::step_count(total),
        ],
    );
    let dialog = adw::AlertDialog::new(Some(&gettext("Replace History?")), Some(&body));
    dialog.add_responses(&[("cancel", &gettext("Cancel")), ("replace", &gettext("Replace"))]);
    dialog.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let target = shared.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "replace" {
            return;
        }
        let result = target.store.borrow_mut().replace_history(&history);
        match result {
            Ok(()) => {
                target.client.reload();
                target.refresh();
                toast(&target, &gettext("History imported"));
            }
            Err(error) => {
                toast(&target, &format::fill(&gettext("Import failed: {}"), &[&error.to_string()]));
            }
        }
    });
    dialog.present(Some(&shared.window));
}

pub fn import(shared: &Shared) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&gettext("History files")));
    filter.add_suffix("csv");
    filter.add_suffix("db");
    filter.add_suffix("sqlite");
    filter.add_pattern("Stepsy*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder().title(gettext("Import History")).filters(&filters).build();
    let shared = shared.clone();
    dialog.open(Some(&shared.window.clone()), gio::Cancellable::NONE, move |file| {
        let Some(path) = file.ok().and_then(|file| file.path()) else {
            return;
        };
        match read_history(&path) {
            Ok(history) => confirm_import(&shared, history),
            Err(error) => toast(&shared, &format::fill(&gettext("Import failed: {}"), &[&error])),
        }
    });
}
