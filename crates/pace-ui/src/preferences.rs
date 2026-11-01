// SPDX-License-Identifier: GPL-3.0-only
//! Preferences dialog. Stored metric; shown in the selected unit system.

use crate::transfer;
use crate::window::Shared;
use adw::prelude::*;
use chrono::Local;
use gettextrs::gettext;
use gtk::gio;
use pace_core::settings::{key, Settings};
use pace_core::units::{CM_PER_INCH, KG_TO_LBS};

pub fn apply_theme(settings: &Settings) {
    adw::StyleManager::default().set_color_scheme(match settings.theme {
        1 => adw::ColorScheme::ForceLight,
        2 => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    });
}

fn save(shared: &Shared, name: &str, value: &str) {
    if let Err(error) = shared.store.borrow().set_setting(name, value) {
        shared.toasts.add_toast(adw::Toast::new(&error.to_string()));
    }
    shared.client.reload();
    shared.refresh();
}

fn switch(shared: &Shared, title: &str, name: &'static str, active: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).active(active).build();
    let shared = shared.clone();
    row.connect_active_notify(move |row| save(&shared, name, &row.is_active().to_string()));
    row
}

fn combo(
    shared: &Shared,
    title: &str,
    name: &'static str,
    items: &[&str],
    codes: &'static [u32],
    current: u32,
) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(items))
        .selected(codes.iter().position(|code| *code == current).unwrap_or(0) as u32)
        .build();
    let shared = shared.clone();
    row.connect_selected_notify(move |row| {
        let code = codes[(row.selected() as usize).min(codes.len() - 1)];
        save(&shared, name, &code.to_string());
        apply_theme(&shared.settings());
    });
    row
}

fn spin(title: &str, minimum: f64, maximum: f64, step: f64, digits: u32, value: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(minimum, maximum, step);
    row.set_title(title);
    row.set_digits(digits);
    row.set_value(value);
    row
}

fn goal_group(shared: &Shared) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(gettext("Daily Goal"))
        .description(gettext("A new goal applies from today. Past days keep their own goal."))
        .build();
    let today = Local::now().date_naive();
    let current = shared.store.borrow().goal_on(today).unwrap_or(10_000);
    let row = spin(&gettext("Steps per day"), 100.0, 200_000.0, 500.0, 0, current as f64);
    let target = shared.clone();
    row.connect_value_notify(move |row| {
        let date = Local::now().date_naive();
        if let Err(error) = target.store.borrow().set_goal(date, row.value() as u64) {
            target.toasts.add_toast(adw::Toast::new(&error.to_string()));
        }
        target.client.reload();
        target.refresh();
    });
    group.add(&row);
    group
}

fn profile_group(shared: &Shared, settings: &Settings) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(gettext("Profile"))
        .description(gettext("Used only for the distance and energy estimates."))
        .build();
    group.add(&switch(shared, &gettext("Imperial units"), key::IMPERIAL, settings.imperial));

    let imperial = settings.imperial;
    let length = if imperial { CM_PER_INCH } else { 1.0 };
    let mass = if imperial { 1.0 / KG_TO_LBS } else { 1.0 };
    let (height_title, weight_title, step_title) = if imperial {
        (gettext("Height (in)"), gettext("Weight (lbs)"), gettext("Step length (in, 0 = from height)"))
    } else {
        (gettext("Height (cm)"), gettext("Weight (kg)"), gettext("Step length (cm, 0 = from height)"))
    };
    let height = spin(&height_title, 40.0 / length, 250.0 / length, 1.0, 1, settings.profile.height_cm / length);
    let weight = spin(&weight_title, 1.0, 500.0 / mass, 1.0, 1, settings.profile.weight_kg / mass);
    let step = spin(&step_title, 0.0, 150.0 / length, 1.0, 1, settings.profile.step_length_cm.unwrap_or(0.0) / length);

    let target = shared.clone();
    height.connect_value_notify(move |row| save(&target, key::HEIGHT_CM, &(row.value() * length).to_string()));
    let target = shared.clone();
    weight.connect_value_notify(move |row| save(&target, key::WEIGHT_KG, &(row.value() * mass).to_string()));
    let target = shared.clone();
    step.connect_value_notify(move |row| {
        if row.value() > 0.0 {
            save(&target, key::STEP_LENGTH_CM, &(row.value() * length).to_string());
        } else {
            if let Err(error) = target.store.borrow().remove_setting(key::STEP_LENGTH_CM) {
                target.toasts.add_toast(adw::Toast::new(&error.to_string()));
            }
            target.refresh();
        }
    });
    group.add(&height);
    group.add(&weight);
    group.add(&step);
    group
}

fn appearance_group(shared: &Shared, settings: &Settings) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(gettext("Appearance")).build();
    let themes = [gettext("System"), gettext("Light"), gettext("Dark")];
    let themes: Vec<&str> = themes.iter().map(String::as_str).collect();
    group.add(&combo(shared, &gettext("Style"), key::THEME, &themes, &[0, 1, 2], settings.theme));
    group.add(&combo(
        shared,
        &gettext("Date format"),
        key::DATE_FORMAT,
        &["2012-12-31", "31.12.2012", "12/31/2012", "31/12/2012", "31-12-2012"],
        &[0, 1, 2, 3, 4],
        settings.date_format as u32,
    ));
    let days = [gettext("Monday"), gettext("Sunday"), gettext("Saturday")];
    let days: Vec<&str> = days.iter().map(String::as_str).collect();
    group.add(&combo(
        shared,
        &gettext("First day of the week"),
        key::FIRST_WEEKDAY,
        &days,
        &[1, 0, 6],
        settings.first_weekday_code,
    ));
    group.add(&switch(
        shared,
        &gettext("Chart the current week"),
        key::CHART_CURRENT_WEEK,
        settings.chart_current_week,
    ));
    group
}

fn notification_group(shared: &Shared, settings: &Settings) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(gettext("Notifications")).build();
    group.add(&switch(shared, &gettext("Daily goal reached"), key::NOTIFY_GOAL, settings.notify_goal));
    group.add(&switch(
        shared,
        &gettext("Encouragement during the day"),
        key::NOTIFY_ENCOURAGE,
        settings.notify_encourage,
    ));
    group.add(&switch(shared, &gettext("New milestones"), key::NOTIFY_MILESTONES, settings.notify_milestones));
    group
}

fn backup_group(shared: &Shared) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(gettext("Automatic Copies"))
        .description(gettext("Copies are written around midnight while the recorder runs."))
        .build();
    let store = shared.store.borrow();
    let directory = store.setting(key::BACKUP_DIRECTORY).ok().flatten().unwrap_or_default();
    let interval: f64 =
        store.setting(key::BACKUP_INTERVAL_DAYS).ok().flatten().and_then(|text| text.parse().ok()).unwrap_or(0.0);
    let retention: f64 =
        store.setting(key::BACKUP_RETENTION).ok().flatten().and_then(|text| text.parse().ok()).unwrap_or(5.0);
    let last_error = store.setting(key::BACKUP_LAST_ERROR).ok().flatten();
    drop(store);

    let folder = adw::ActionRow::builder()
        .title(gettext("Folder"))
        .subtitle(if directory.is_empty() { gettext("Not selected") } else { directory })
        .activatable(true)
        .build();
    let target = shared.clone();
    folder.connect_activated(move |row| {
        let dialog = gtk::FileDialog::builder().title(gettext("Select a Folder")).build();
        let target = target.clone();
        let row = row.clone();
        dialog.select_folder(Some(&target.window.clone()), gio::Cancellable::NONE, move |folder| {
            if let Some(path) = folder.ok().and_then(|folder| folder.path()) {
                let text = path.to_string_lossy();
                save(&target, key::BACKUP_DIRECTORY, &text);
                row.set_subtitle(&text);
            }
        });
    });
    group.add(&folder);

    let every = spin(&gettext("Every N days (0 = off)"), 0.0, 365.0, 1.0, 0, interval);
    let target = shared.clone();
    every.connect_value_notify(move |row| save(&target, key::BACKUP_INTERVAL_DAYS, &(row.value() as u32).to_string()));
    group.add(&every);

    let keep = spin(&gettext("Copies to keep (0 = all)"), 0.0, 1000.0, 1.0, 0, retention);
    let target = shared.clone();
    keep.connect_value_notify(move |row| save(&target, key::BACKUP_RETENTION, &(row.value() as u32).to_string()));
    group.add(&keep);

    if let Some(error) = last_error {
        group.add(&adw::ActionRow::builder().title(gettext("Last copy failed")).subtitle(error).build());
    }
    group
}

fn transfer_group(shared: &Shared, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(gettext("History Files"))
        .description(gettext("Import accepts CSV files and copies of a Stepsy database."))
        .build();
    let export = adw::ActionRow::builder().title(gettext("Export History")).activatable(true).build();
    let target = shared.clone();
    export.connect_activated(move |_| transfer::export(&target));
    let import = adw::ActionRow::builder().title(gettext("Import History")).activatable(true).build();
    let target = shared.clone();
    let owner = dialog.clone();
    import.connect_activated(move |_| {
        owner.close();
        transfer::import(&target);
    });
    group.add(&export);
    group.add(&import);
    group
}

pub fn present(shared: &Shared) {
    let settings = shared.settings();
    let dialog = adw::PreferencesDialog::new();

    let general =
        adw::PreferencesPage::builder().title(gettext("General")).icon_name("preferences-system-symbolic").build();
    general.add(&goal_group(shared));
    general.add(&profile_group(shared, &settings));
    general.add(&appearance_group(shared, &settings));
    general.add(&notification_group(shared, &settings));

    let data = adw::PreferencesPage::builder().title(gettext("Data")).icon_name("drive-harddisk-symbolic").build();
    data.add(&backup_group(shared));
    data.add(&transfer_group(shared, &dialog));

    dialog.add(&general);
    dialog.add(&data);
    dialog.present(Some(&shared.window));
}
