// SPDX-License-Identifier: GPL-3.0-only
//! Main window and its three pages.

use crate::chart::Chart;
use crate::client::{Client, RecorderState};
use crate::format;
use crate::preferences;
use adw::prelude::*;
use chrono::{Days, Local, NaiveDate};
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use pace_core::csv::History;
use pace_core::settings::{PauseState, Settings};
use pace_core::stats::{self, GoalHistory, Period};
use pace_core::store::{default_path, Store};
use pace_core::units::plural_count;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const MINIMUM_STREAK_SHOWN: u32 = 2;
const PERIODS: [Period; 7] =
    [Period::Today, Period::Week, Period::Month, Period::Last7, Period::Last30, Period::Year, Period::AllTime];

#[derive(Clone)]
pub struct Shared(Rc<Inner>);

pub struct Inner {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub store: RefCell<Store>,
    pub client: Client,
    state: RefCell<Option<RecorderState>>,
    today: Today,
    history: HistoryPage,
    achievements: gtk::Box,
    banner: adw::Banner,
    period: Cell<usize>,
}

impl std::ops::Deref for Shared {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

struct Today {
    steps: gtk::Label,
    goal: gtk::Label,
    progress: gtk::LevelBar,
    distance: adw::ActionRow,
    energy: adw::ActionRow,
    streak: adw::ActionRow,
    pause: gtk::Button,
    edit: gtk::Button,
    chart: Chart,
}

struct HistoryPage {
    periods: gtk::DropDown,
    total: adw::ActionRow,
    average: adw::ActionRow,
    distance: adw::ActionRow,
    energy: adw::ActionRow,
    chart: Chart,
    calendar: gtk::Calendar,
    selected: adw::ActionRow,
}

fn value_row(title: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle_selectable(true).build();
    row.add_css_class("property");
    row
}

fn page(content: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .child(content)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build()
}

fn column() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 18)
}

fn list(rows: &[&adw::ActionRow]) -> gtk::ListBox {
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list.add_css_class("boxed-list");
    for row in rows {
        list.append(*row);
    }
    list
}

impl Today {
    fn new() -> (Self, gtk::Box) {
        let steps = gtk::Label::builder().css_classes(["title-1"]).build();
        steps.set_attributes(Some(&{
            let attributes = gtk::pango::AttrList::new();
            attributes.insert(gtk::pango::AttrFloat::new_scale(2.4));
            attributes
        }));
        let goal = gtk::Label::builder().css_classes(["dim-label"]).build();
        let progress = gtk::LevelBar::builder().min_value(0.0).max_value(1.0).build();
        progress.update_property(&[gtk::accessible::Property::Label(&gettext("Progress to the daily goal"))]);
        let distance = value_row(&gettext("Distance (estimate)"));
        let energy = value_row(&gettext("Energy (estimate)"));
        let streak = value_row(&gettext("Streak"));
        let pause = gtk::Button::builder().hexpand(true).css_classes(["pill"]).build();
        let edit = gtk::Button::builder().label(gettext("Edit Steps")).hexpand(true).css_classes(["pill"]).build();
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        controls.append(&pause);
        controls.append(&edit);
        let chart = Chart::new();

        let content = column();
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 6);
        heading.append(&steps);
        heading.append(&goal);
        heading.append(&progress);
        content.append(&heading);
        content.append(&controls);
        content.append(&list(&[&distance, &energy, &streak]));
        content.append(&chart.widget);
        (Self { steps, goal, progress, distance, energy, streak, pause, edit, chart }, content)
    }
}

impl HistoryPage {
    fn new() -> (Self, gtk::Box) {
        let names = [
            gettext("Today"),
            gettext("This week"),
            gettext("This month"),
            gettext("Last 7 days"),
            gettext("Last 30 days"),
            gettext("This year"),
            gettext("All time"),
        ];
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let periods = gtk::DropDown::from_strings(&names);
        periods.set_selected(3);
        periods.update_property(&[gtk::accessible::Property::Label(&gettext("Period"))]);
        let total = value_row(&gettext("Total"));
        let average = value_row(&gettext("Daily average"));
        let distance = value_row(&gettext("Distance (estimate)"));
        let energy = value_row(&gettext("Energy (estimate)"));
        let chart = Chart::new();
        let calendar = gtk::Calendar::new();
        let selected = value_row(&gettext("Selected day"));

        let content = column();
        content.append(&periods);
        content.append(&chart.widget);
        content.append(&list(&[&total, &average, &distance, &energy]));
        content.append(&calendar);
        content.append(&list(&[&selected]));
        (Self { periods, total, average, distance, energy, chart, calendar, selected }, content)
    }
}

fn goal_label(goal: u64) -> String {
    format::fill(&ngettext("Goal: {} step", "Goal: {} steps", plural_count(goal)), &[&format::steps(goal)])
}

impl Shared {
    pub fn settings(&self) -> Settings {
        self.store.borrow().settings().unwrap_or_else(|error| {
            eprintln!("settings: {error}");
            Store::open_in_memory().and_then(|store| store.settings()).expect("defaults")
        })
    }

    fn today_date(&self) -> NaiveDate {
        Local::now().date_naive()
    }

    /// Stored history with today's live total from the recorder.
    fn history_with_today(&self) -> History {
        let mut history = self.store.borrow().history().unwrap_or_default();
        if let Some(state) = self.state.borrow().as_ref() {
            let today = self.today_date();
            if state.today_steps > 0 || history.contains_key(&today) {
                history.insert(today, state.today_steps);
            }
        }
        history
    }

    fn goals(&self) -> GoalHistory {
        self.store.borrow().goals().unwrap_or_default()
    }

    pub fn refresh(&self) {
        let settings = self.settings();
        let history = self.history_with_today();
        let goals = self.goals();
        self.refresh_today(&settings, &history, &goals);
        self.refresh_history(&settings, &history);
        self.refresh_achievements(&settings, &history, &goals);
        self.refresh_status();
    }

    fn refresh_today(&self, settings: &Settings, history: &History, goals: &GoalHistory) {
        let date = self.today_date();
        let steps = history.get(&date).copied().unwrap_or(0);
        let goal = goals.goal_on(date);
        let page = &self.today;
        page.steps.set_label(&format::steps(steps));
        page.goal.set_label(&goal_label(goal));
        page.progress.set_value((steps as f64 / goal as f64).min(1.0));
        page.distance.set_subtitle(&format::distance(settings, steps));
        page.energy.set_subtitle(&format::energy(settings, steps));

        let streak = stats::current_streak(history, goals, date);
        page.streak.set_visible(streak >= MINIMUM_STREAK_SHOWN);
        page.streak.set_subtitle(&format::day_count(u64::from(streak)));

        let first = if settings.chart_current_week {
            stats::week_start(date, settings.first_weekday)
        } else {
            date.checked_sub_days(Days::new(6)).unwrap_or(date)
        };
        let (resolution, bars) = stats::buckets(history, first, date);
        page.chart.set_data(resolution, bars, Some(goal));
    }

    fn refresh_history(&self, settings: &Settings, history: &History) {
        let today = self.today_date();
        let period = PERIODS[self.period.get().min(PERIODS.len() - 1)];
        let (first, last) = stats::period_range(period, history, today, settings.first_weekday);
        let summary = stats::summarize(history, first, last);
        let page = &self.history;
        page.total.set_subtitle(&format::steps(summary.total));
        page.average.set_subtitle(&format::steps(summary.average));
        page.distance.set_subtitle(&format::distance(settings, summary.total));
        page.energy.set_subtitle(&format::energy(settings, summary.total));
        let (resolution, bars) = stats::buckets(history, first, last);
        page.chart.set_data(resolution, bars, None);

        let selection = page.calendar.date();
        let selected =
            NaiveDate::from_ymd_opt(selection.year(), selection.month() as u32, selection.day_of_month() as u32);
        if let Some(selected) = selected {
            let steps = history.get(&selected).copied().unwrap_or(0);
            page.selected.set_title(&format::date(settings, selected));
            page.selected.set_subtitle(&format::step_count(steps));
        }
    }

    fn refresh_achievements(&self, settings: &Settings, history: &History, goals: &GoalHistory) {
        while let Some(child) = self.achievements.first_child() {
            self.achievements.remove(&child);
        }
        let results = stats::achievements(history, goals, settings.first_weekday);
        let Some(first_date) = results.first_date else {
            self.achievements.append(
                &adw::StatusPage::builder()
                    .title(gettext("No Records Yet"))
                    .description(gettext("Achievements appear after the first walk."))
                    .build(),
            );
            return;
        };
        let group = |title: String| adw::PreferencesGroup::builder().title(title).build();
        let row = |title: String, subtitle: String| {
            adw::ActionRow::builder().title(title).subtitle(subtitle).css_classes(["property"]).build()
        };
        let span = |first, last| {
            format::fill(&gettext("{} to {}"), &[&format::date(settings, first), &format::date(settings, last)])
        };

        let records = group(gettext("Records"));
        records.add(&row(gettext("Recording since"), format::date(settings, first_date)));
        records.add(&row(gettext("Total steps"), format::steps(results.total)));
        records.add(&row(gettext("Average per recorded day"), format::steps(results.average)));
        if let Some(week) = results.best_week {
            records.add(&row(
                gettext("Best week"),
                format!("{} · {}", format::steps(week.steps), span(week.first, week.last)),
            ));
        }
        if let Some(month) = results.best_month {
            records.add(&row(
                gettext("Best month"),
                format!("{} · {:02}/{}", format::steps(month.steps), month.month, month.year),
            ));
        }
        if let Some(streak) = results.longest_streak {
            records.add(&row(
                gettext("Longest streak"),
                format!("{} · {}", format::day_count(u64::from(streak.days)), span(streak.first, streak.last)),
            ));
        }
        self.achievements.append(&records);

        let top = group(gettext("Top Days"));
        for day in &results.top_days {
            top.add(&row(format::date(settings, day.date), format::steps(day.steps)));
        }
        self.achievements.append(&top);

        let milestones = group(gettext("Milestones"));
        for threshold in stats::milestones() {
            let reached = results.milestones.iter().find(|m| m.threshold == threshold);
            let item = adw::ActionRow::builder()
                .title(format::steps(threshold))
                .subtitle(
                    reached
                        .map_or_else(|| gettext("Not reached yet"), |milestone| format::date(settings, milestone.date)),
                )
                .sensitive(reached.is_some())
                .build();
            item.add_prefix(&gtk::Label::new(Some(stats::milestone_badge(threshold))));
            milestones.add(&item);
        }
        self.achievements.append(&milestones);
    }

    fn refresh_status(&self) {
        let state = self.state.borrow();
        let paused = state.as_ref().map(|state| PauseState::decode(&state.pause));
        let today = &self.today;
        today.pause.set_sensitive(state.is_some());
        today.edit.set_sensitive(state.is_some());
        today.pause.set_label(&if paused.is_some_and(PauseState::is_paused) {
            gettext("Resume")
        } else {
            gettext("Pause")
        });
        let notice = match (state.as_ref(), paused) {
            (None, _) => Some(gettext("The recorder is not running. Steps are not being counted.")),
            (Some(state), _) if !state.sensor_connected => Some(gettext("Waiting for the step sensor.")),
            (_, Some(PauseState::Indefinite)) => Some(gettext("Counting is paused.")),
            (_, Some(PauseState::Until(moment))) => chrono::DateTime::from_timestamp(moment, 0)
                .map(|time| time.with_timezone(&Local).format("%H:%M").to_string())
                .map(|time| format::fill(&gettext("Counting is paused until {}."), &[&time])),
            _ => None,
        };
        self.banner.set_revealed(notice.is_some());
        if let Some(notice) = notice {
            self.banner.set_title(&notice);
        }
    }

    fn apply_state(&self, state: Option<RecorderState>) {
        *self.state.borrow_mut() = state;
        self.refresh();
    }

    fn poll_state(&self) {
        let shared = self.clone();
        self.client.state(move |state| shared.apply_state(state));
    }

    fn ask_pause(&self) {
        let paused = self.state.borrow().as_ref().is_some_and(|state| PauseState::decode(&state.pause).is_paused());
        if paused {
            let shared = self.clone();
            self.client.resume(move |_| shared.poll_state());
            return;
        }
        let custom = adw::SpinRow::with_range(1.0, 1440.0, 5.0);
        custom.set_title(&gettext("Minutes"));
        custom.set_value(15.0);
        let extra = list_of(&custom);
        let dialog = adw::AlertDialog::new(Some(&gettext("Pause Counting")), None);
        dialog.set_extra_child(Some(&extra));
        dialog.add_responses(&[
            ("cancel", &gettext("Cancel")),
            ("custom", &gettext("For Minutes Above")),
            ("30", &gettext("30 Minutes")),
            ("60", &gettext("1 Hour")),
            ("120", &gettext("2 Hours")),
            ("0", &gettext("Until Resumed")),
        ]);
        dialog.set_prefer_wide_layout(false);
        dialog.set_close_response("cancel");
        let shared = self.clone();
        dialog.connect_response(None, move |_, response| {
            let minutes = match response {
                "cancel" => return,
                "custom" => custom.value() as u32,
                other => other.parse().unwrap_or(0),
            };
            let target = shared.clone();
            shared.client.pause(minutes, move |_| target.poll_state());
        });
        dialog.present(Some(&self.window));
    }

    fn ask_edit(&self) {
        let current = self.state.borrow().as_ref().map_or(0, |state| state.today_steps);
        let spin = adw::SpinRow::with_range(0.0, 1_000_000.0, 100.0);
        spin.set_title(&gettext("Steps today"));
        spin.set_value(current as f64);
        let dialog = adw::AlertDialog::new(Some(&gettext("Edit Today's Steps")), None);
        dialog.set_extra_child(Some(&list_of(&spin)));
        dialog.add_responses(&[("cancel", &gettext("Cancel")), ("save", &gettext("Save"))]);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_close_response("cancel");
        let shared = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "save" {
                return;
            }
            let steps = spin.value() as u64;
            if steps < current {
                shared.confirm_decrease(steps);
            } else {
                shared.save_today(steps);
            }
        });
        dialog.present(Some(&self.window));
    }

    fn confirm_decrease(&self, steps: u64) {
        let dialog = adw::AlertDialog::new(
            Some(&gettext("Reduce Today's Steps?")),
            Some(&format::fill(&gettext("Today's total will become {}."), &[&format::steps(steps)])),
        );
        dialog.add_responses(&[("cancel", &gettext("Cancel")), ("reduce", &gettext("Reduce"))]);
        dialog.set_response_appearance("reduce", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        let shared = self.clone();
        dialog.connect_response(Some("reduce"), move |_, _| shared.save_today(steps));
        dialog.present(Some(&self.window));
    }

    fn save_today(&self, steps: u64) {
        let shared = self.clone();
        self.client.set_today_steps(steps, move |saved| {
            if !saved {
                shared.toasts.add_toast(adw::Toast::new(&gettext("The recorder did not accept the change")));
            }
            shared.poll_state();
        });
    }
}

fn list_of(row: &impl IsA<gtk::Widget>) -> gtk::ListBox {
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list.add_css_class("boxed-list");
    list.append(row);
    list
}

fn install_actions(shared: &Shared, application: &adw::Application) {
    let preferences = gio::SimpleAction::new("preferences", None);
    let target = shared.clone();
    preferences.connect_activate(move |_, _| preferences::present(&target));
    application.add_action(&preferences);

    let about = gio::SimpleAction::new("about", None);
    let target = shared.clone();
    about.connect_activate(move |_, _| {
        adw::AboutDialog::builder()
            .application_name("Pace Beads")
            .application_icon(crate::APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .license_type(gtk::License::Gpl30Only)
            .comments(gettext("An offline step counter."))
            .website("https://github.com/x-vlad-x/pace-beads")
            .developer_name("Vladislav Shadiuk")
            .build()
            .present(Some(&target.window));
    });
    application.add_action(&about);
    application.set_accels_for_action("app.preferences", &["<Control>comma"]);
}

fn menu_button() -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append(Some(&gettext("Preferences")), Some("app.preferences"));
    menu.append(Some(&gettext("About Pace Beads")), Some("app.about"));
    gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text(gettext("Main Menu"))
        .primary(true)
        .build()
}

pub fn build(application: &adw::Application) {
    let store = match Store::open(&default_path()) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("cannot open the history: {error}");
            let dialog = adw::AlertDialog::new(Some(&gettext("Cannot Open the History")), Some(&error.to_string()));
            dialog.add_response("close", &gettext("Close"));
            let window = adw::ApplicationWindow::new(application);
            window.present();
            dialog.present(Some(&window));
            return;
        }
    };

    let (today, today_content) = Today::new();
    let (history, history_content) = HistoryPage::new();
    let achievements = column();

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(&page(&today_content), Some("today"), &gettext("Today"), "go-home-symbolic");
    stack.add_titled_with_icon(
        &page(&history_content),
        Some("history"),
        &gettext("History"),
        "x-office-calendar-symbolic",
    );
    stack.add_titled_with_icon(
        &page(&achievements),
        Some("achievements"),
        &gettext("Achievements"),
        "starred-symbolic",
    );

    let header = adw::HeaderBar::new();
    header.pack_end(&menu_button());
    let switcher = adw::ViewSwitcherBar::builder().stack(&stack).reveal(true).build();
    let banner = adw::Banner::new("");
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&stack));

    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.add_top_bar(&banner);
    content.add_bottom_bar(&switcher);
    content.set_content(Some(&toasts));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Pace Beads")
        .default_width(420)
        .default_height(760)
        .width_request(320)
        .height_request(320)
        .content(&content)
        .build();

    let shared = Shared(Rc::new(Inner {
        window: window.clone(),
        toasts,
        store: RefCell::new(store),
        client: Client::connect(),
        state: RefCell::new(None),
        today,
        history,
        achievements,
        banner,
        period: Cell::new(3),
    }));

    install_actions(&shared, application);
    preferences::apply_theme(&shared.settings());

    let target = shared.clone();
    shared.today.pause.connect_clicked(move |_| target.ask_pause());
    let target = shared.clone();
    shared.today.edit.connect_clicked(move |_| target.ask_edit());
    let target = shared.clone();
    shared.history.periods.connect_selected_notify(move |dropdown| {
        target.period.set(dropdown.selected() as usize);
        target.refresh();
    });
    let target = shared.clone();
    shared.history.calendar.connect_day_selected(move |_| target.refresh());

    let target = shared.clone();
    shared.client.subscribe(move |state| target.apply_state(Some(state)));
    shared.refresh();
    shared.poll_state();

    // Catches the recorder appearing or leaving, and the change of date.
    let target = shared.clone();
    let weak = window.downgrade();
    glib::timeout_add_seconds_local(30, move || {
        if weak.upgrade().is_none_or(|window| !window.is_visible()) {
            return glib::ControlFlow::Continue;
        }
        target.poll_state();
        glib::ControlFlow::Continue
    });

    window.present();
}
