// SPDX-License-Identifier: GPL-3.0-only
//! Background recorder: holds the sensor session and owns the step count.

mod sensorfw;

use chrono::{DateTime, Duration as ChronoDuration, Local};
use gettextrs::{gettext, ngettext};
use glib::variant::ToVariant;
use glib::{ControlFlow, IOCondition};
use pace_core::backup;
use pace_core::engine::{Alert, Engine};
use pace_core::ipc;
use pace_core::store::{default_path, Store};
use pace_core::units::{group_digits, plural_count};
use sensorfw::Session;
use std::cell::RefCell;
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

const MAX_RETRY_SECONDS: u32 = 60;
const STATE_SIGNAL_INTERVAL: Duration = Duration::from_secs(2);
const APP_ICON: &str = "io.github.x_vlad_x.PaceBeads";

struct Recorder {
    engine: Engine,
    session: Option<Session>,
    watch: Option<glib::SourceId>,
    timer: Option<glib::SourceId>,
    retry_seconds: u32,
    bus: Option<gio::DBusConnection>,
    last_signal: Option<Instant>,
    signal_timer: Option<glib::SourceId>,
}

type Shared = Rc<RefCell<Recorder>>;

fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map(|text| text.trim().to_owned()).unwrap_or_default()
}

fn uptime_seconds() -> u64 {
    std::fs::read_to_string("/proc/uptime").ok().and_then(|text| text.split('.').next()?.parse().ok()).unwrap_or(0)
}

fn report<T, E: std::fmt::Display>(what: &str, result: Result<T, E>) -> Option<T> {
    result.map_err(|error| eprintln!("{what}: {error}")).ok()
}

fn state_tuple(recorder: &Recorder) -> (u64, String, bool) {
    (recorder.engine.today_steps(), recorder.engine.pause_state().encode(), recorder.session.is_some())
}

fn emit_state(recorder: &mut Recorder, force: bool) {
    let due = recorder.last_signal.is_none_or(|time| time.elapsed() >= STATE_SIGNAL_INTERVAL);
    if !(force || due) {
        return;
    }
    let Some(bus) = recorder.bus.clone() else {
        return;
    };
    recorder.last_signal = Some(Instant::now());
    let state = state_tuple(recorder);
    report(
        "state signal",
        bus.emit_signal(
            None::<&str>,
            ipc::OBJECT_PATH,
            ipc::INTERFACE,
            "StateChanged",
            Some(&ToVariant::to_variant(&state)),
        ),
    );
}

/// Makes sure a reading held back by the rate limit still reaches the clients.
fn schedule_state(shared: &Shared) {
    let mut recorder = shared.borrow_mut();
    let unsent = recorder.last_signal.is_some_and(|time| time.elapsed() < STATE_SIGNAL_INTERVAL);
    if !unsent || recorder.signal_timer.is_some() {
        return;
    }
    let weak = Rc::downgrade(shared);
    recorder.signal_timer = Some(glib::timeout_add_local_once(STATE_SIGNAL_INTERVAL, move || {
        if let Some(shared) = weak.upgrade() {
            let mut recorder = shared.borrow_mut();
            recorder.signal_timer = None;
            emit_state(&mut recorder, true);
        }
    }));
}

fn notify(bus: &gio::DBusConnection, title: &str, body: &str) {
    let hints = glib::VariantDict::new(None);
    hints.insert("desktop-entry", APP_ICON);
    // A plain tuple would box the hints as a single variant value.
    let arguments = glib::Variant::tuple_from_iter([
        "Pace Beads".to_variant(),
        0u32.to_variant(),
        APP_ICON.to_variant(),
        title.to_variant(),
        body.to_variant(),
        Vec::<String>::new().to_variant(),
        hints.end(),
        (-1i32).to_variant(),
    ]);
    bus.call(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        Some(&arguments),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
        |result| {
            report("notification", result);
        },
    );
}

fn send_alerts(recorder: &Recorder, alerts: &[Alert]) {
    let Some(bus) = recorder.bus.as_ref() else {
        return;
    };
    for alert in alerts {
        match alert {
            Alert::GoalReached { goal } => notify(
                bus,
                &gettext("Daily goal reached"),
                &ngettext("You walked {} step today.", "You walked {} steps today.", plural_count(*goal)).replacen(
                    "{}",
                    &group_digits(*goal),
                    1,
                ),
            ),
            Alert::Encouragement { percent: 15 } => {
                notify(bus, &gettext("Good start"), &gettext("You are on your way to today's goal."))
            }
            Alert::Encouragement { .. } => {
                notify(bus, &gettext("Almost there"), &gettext("Three quarters of today's goal are done."))
            }
            Alert::Milestone { threshold } => {
                notify(
                    bus,
                    &gettext("New milestone"),
                    &ngettext("{} step walked in total.", "{} steps walked in total.", plural_count(*threshold))
                        .replacen("{}", &group_digits(*threshold), 1),
                )
            }
        }
    }
}

fn accept_reading(recorder: &mut Recorder, value: u32, now: DateTime<Local>) {
    if let Some(alerts) = report("sample", recorder.engine.on_sample(value, now, uptime_seconds())) {
        send_alerts(recorder, &alerts);
    }
    emit_state(recorder, false);
}

fn seconds_until_midnight(now: DateTime<Local>) -> u32 {
    let tomorrow = now.date_naive() + ChronoDuration::days(1);
    let midnight = tomorrow.and_hms_opt(0, 0, 1).unwrap_or(now.naive_local());
    (midnight - now.naive_local()).num_seconds().clamp(1, 86_400) as u32
}

fn reschedule(shared: &Shared) {
    let now = Local::now();
    let mut recorder = shared.borrow_mut();
    if let Some(source) = recorder.timer.take() {
        source.remove();
    }
    let mut delay = seconds_until_midnight(now);
    if let Some(deadline) = recorder.engine.flush_deadline() {
        delay = delay.min((deadline - now.timestamp()).clamp(1, i64::from(delay)) as u32);
    }
    if let pace_core::settings::PauseState::Until(moment) = recorder.engine.pause_state() {
        delay = delay.min((moment - now.timestamp()).clamp(1, i64::from(delay)) as u32);
    }
    let weak = Rc::downgrade(shared);
    recorder.timer = Some(glib::timeout_add_seconds_local_once(delay, move || {
        if let Some(shared) = weak.upgrade() {
            shared.borrow_mut().timer = None;
            maintain(&shared);
        }
    }));
}

/// Applies everything that depends on the clock, then plans the next wake-up.
fn maintain(shared: &Shared) {
    {
        let now = Local::now();
        let mut guard = shared.borrow_mut();
        let recorder = &mut *guard;
        let needs_reading = recorder.engine.pause_state().expired(now);
        if needs_reading {
            if let Some(value) = recorder.session.as_ref().and_then(Session::current_value) {
                accept_reading(recorder, value, now);
            }
        }
        report("day change", recorder.engine.roll_day(now));
        report("pause timer", recorder.engine.tick(now));
        report("flush", recorder.engine.flush(now));
        report("backup", backup::run_if_due(recorder.engine.store(), now));
        emit_state(recorder, true);
    }
    reschedule(shared);
}

fn drop_session(shared: &Shared, delay_seconds: u32) {
    {
        let mut recorder = shared.borrow_mut();
        if let Some(source) = recorder.watch.take() {
            source.remove();
        }
        recorder.session = None;
        emit_state(&mut recorder, true);
    }
    let weak = Rc::downgrade(shared);
    glib::timeout_add_seconds_local_once(delay_seconds, move || {
        if let Some(shared) = weak.upgrade() {
            connect_sensor(&shared);
        }
    });
}

fn on_socket(shared: &Shared) -> ControlFlow {
    let now = Local::now();
    let readings = {
        let mut recorder = shared.borrow_mut();
        recorder.session.as_mut().and_then(Session::read_available)
    };
    match readings {
        Some(readings) => {
            let mut recorder = shared.borrow_mut();
            for reading in readings {
                accept_reading(&mut recorder, reading.value, now);
            }
            drop(recorder);
            schedule_state(shared);
            reschedule(shared);
            ControlFlow::Continue
        }
        None => {
            eprintln!("sensor connection closed");
            shared.borrow_mut().watch = None;
            let weak = Rc::downgrade(shared);
            glib::idle_add_local_once(move || {
                if let Some(shared) = weak.upgrade() {
                    drop_session(&shared, 1);
                }
            });
            ControlFlow::Break
        }
    }
}

fn connect_sensor(shared: &Shared) {
    match Session::open() {
        Ok(session) => {
            eprintln!("sensor session {} started", session.session_id());
            let fd = session.stream.as_raw_fd();
            let weak = Rc::downgrade(shared);
            let watch =
                glib_unix::unix_fd_add_local(fd, IOCondition::IN | IOCondition::HUP | IOCondition::ERR, move |_, _| {
                    weak.upgrade().map_or(ControlFlow::Break, |shared| on_socket(&shared))
                });
            let mut guard = shared.borrow_mut();
            let recorder = &mut *guard;
            recorder.retry_seconds = 1;
            let current = session.current_value();
            recorder.session = Some(session);
            recorder.watch = Some(watch);
            if let Some(value) = current {
                accept_reading(recorder, value, Local::now());
            }
            emit_state(recorder, true);
        }
        Err(error) => {
            eprintln!("sensor unavailable: {error}");
            let delay = {
                let mut recorder = shared.borrow_mut();
                let delay = recorder.retry_seconds;
                recorder.retry_seconds = (delay * 2).min(MAX_RETRY_SECONDS);
                delay
            };
            drop_session(shared, delay);
        }
    }
}

fn watch_sleep(shared: &Shared) {
    let Some(connection) = report("system bus", gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)) else {
        return;
    };
    let weak = Rc::downgrade(shared);
    let subscription = connection.subscribe_to_signal(
        Some("org.freedesktop.login1"),
        Some("org.freedesktop.login1.Manager"),
        Some("PrepareForSleep"),
        Some("/org/freedesktop/login1"),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            if signal.parameters.get::<(bool,)>() == Some((false,)) {
                // Timers stop in suspend, and no event reports the steps walked.
                let now = Local::now();
                {
                    let mut guard = shared.borrow_mut();
                    let recorder = &mut *guard;
                    if let Some(value) = recorder.session.as_ref().and_then(Session::current_value) {
                        accept_reading(recorder, value, now);
                    }
                }
                maintain(&shared);
            } else {
                let mut recorder = shared.borrow_mut();
                report("flush before sleep", recorder.engine.flush(Local::now()));
            }
        },
    );
    std::mem::forget(subscription);
}

fn handle_method(
    shared: &Shared,
    method: &str,
    parameters: &glib::Variant,
) -> Result<Option<glib::Variant>, glib::Error> {
    let invalid = || glib::Error::new(gio::DBusError::InvalidArgs, "invalid arguments");
    let now = Local::now();
    let mut reply = None;
    {
        let mut guard = shared.borrow_mut();
        let recorder = &mut *guard;
        let outcome = match method {
            "GetState" => {
                reply = Some(ToVariant::to_variant(&state_tuple(recorder)));
                Ok(())
            }
            "Pause" => {
                let (minutes,) = parameters.get::<(u32,)>().ok_or_else(invalid)?;
                let until = (minutes > 0).then(|| now.timestamp() + i64::from(minutes) * 60);
                recorder.engine.pause(until, now)
            }
            "Resume" => recorder.engine.resume(now),
            "SetTodaySteps" => {
                let (steps,) = parameters.get::<(u64,)>().ok_or_else(invalid)?;
                recorder.engine.set_today(steps, now)
            }
            "Reload" => recorder.engine.reload(now),
            other => {
                return Err(glib::Error::new(gio::DBusError::UnknownMethod, other));
            }
        };
        outcome.map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string()))?;
    }
    if method != "GetState" {
        emit_state(&mut shared.borrow_mut(), true);
        reschedule(shared);
    }
    Ok(reply)
}

fn own_name(shared: &Shared, main_loop: &glib::MainLoop) {
    let acquired = shared.clone();
    let on_bus = shared.clone();
    let quit = main_loop.clone();
    gio::bus_own_name(
        gio::BusType::Session,
        ipc::BUS_NAME,
        gio::BusNameOwnerFlags::NONE,
        move |connection, _| {
            let info = gio::DBusNodeInfo::for_xml(ipc::INTROSPECTION_XML).expect("valid XML");
            let interface = info.lookup_interface(ipc::INTERFACE).expect("declared interface");
            let shared = on_bus.clone();
            let registered = connection
                .register_object(ipc::OBJECT_PATH, &interface)
                .method_call(move |_, _, _, _, method, parameters, invocation| {
                    invocation.return_result(handle_method(&shared, method, &parameters));
                })
                .build();
            report("object registration", registered);
            on_bus.borrow_mut().bus = Some(connection);
        },
        move |_, _| {
            let mut recorder = acquired.borrow_mut();
            emit_state(&mut recorder, true);
        },
        move |_, _| {
            eprintln!("the recorder bus name is unavailable");
            quit.quit();
        },
    );
}

fn main() -> std::process::ExitCode {
    if std::env::args().any(|argument| argument == "--version") {
        println!("pace-beads-recorder {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    let _ = gettextrs::bindtextdomain("pace-beads", pace_core::ipc::locale_directory());
    let _ = gettextrs::bind_textdomain_codeset("pace-beads", "UTF-8");
    let _ = gettextrs::textdomain("pace-beads");

    let now = Local::now();
    let engine = match Store::open(&default_path()).and_then(|store| Engine::new(store, boot_id(), now)) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("cannot open the history: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let shared: Shared = Rc::new(RefCell::new(Recorder {
        engine,
        session: None,
        watch: None,
        timer: None,
        retry_seconds: 1,
        bus: None,
        last_signal: None,
        signal_timer: None,
    }));

    let main_loop = glib::MainLoop::new(None, false);
    for signal in [libc_signal::SIGTERM, libc_signal::SIGINT] {
        let shared = shared.clone();
        let main_loop = main_loop.clone();
        glib_unix::unix_signal_add_local(signal, move || {
            report("final flush", shared.borrow_mut().engine.flush(Local::now()));
            main_loop.quit();
            ControlFlow::Break
        });
    }

    own_name(&shared, &main_loop);
    watch_sleep(&shared);
    connect_sensor(&shared);
    maintain(&shared);
    main_loop.run();
    std::process::ExitCode::SUCCESS
}

mod libc_signal {
    pub const SIGINT: i32 = 2;
    pub const SIGTERM: i32 = 15;
}
