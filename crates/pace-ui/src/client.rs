// SPDX-License-Identifier: GPL-3.0-only
//! Session-bus client for the recorder.

use gtk::gio;
use gtk::glib;
use gtk::glib::variant::ToVariant;
use pace_core::ipc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderState {
    pub today_steps: u64,
    pub pause: String,
    pub sensor_connected: bool,
}

#[derive(Clone)]
pub struct Client {
    connection: Option<gio::DBusConnection>,
}

const TIMEOUT_MS: i32 = 5000;

impl Client {
    pub fn connect() -> Self {
        Self { connection: gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok() }
    }

    fn call(
        &self,
        method: &str,
        arguments: Option<glib::Variant>,
        done: impl FnOnce(Result<glib::Variant, glib::Error>) + 'static,
    ) {
        let Some(connection) = &self.connection else {
            done(Err(glib::Error::new(gio::IOErrorEnum::NotConnected, "no session bus")));
            return;
        };
        connection.call(
            Some(ipc::BUS_NAME),
            ipc::OBJECT_PATH,
            ipc::INTERFACE,
            method,
            arguments.as_ref(),
            None,
            gio::DBusCallFlags::NONE,
            TIMEOUT_MS,
            gio::Cancellable::NONE,
            done,
        );
    }

    fn decode(reply: &glib::Variant) -> Option<RecorderState> {
        let (today_steps, pause, sensor_connected) = reply.get::<(u64, String, bool)>()?;
        Some(RecorderState { today_steps, pause, sensor_connected })
    }

    pub fn state(&self, done: impl FnOnce(Option<RecorderState>) + 'static) {
        self.call("GetState", None, move |reply| {
            done(reply.ok().as_ref().and_then(Self::decode));
        });
    }

    pub fn subscribe(&self, handler: impl Fn(RecorderState) + 'static) {
        let Some(connection) = &self.connection else {
            return;
        };
        let subscription = connection.subscribe_to_signal(
            Some(ipc::BUS_NAME),
            Some(ipc::INTERFACE),
            Some("StateChanged"),
            Some(ipc::OBJECT_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let Some(state) = Self::decode(signal.parameters) {
                    handler(state);
                }
            },
        );
        std::mem::forget(subscription);
    }

    pub fn pause(&self, minutes: u32, done: impl FnOnce(bool) + 'static) {
        self.call("Pause", Some((minutes,).to_variant()), move |reply| done(reply.is_ok()));
    }

    pub fn resume(&self, done: impl FnOnce(bool) + 'static) {
        self.call("Resume", None, move |reply| done(reply.is_ok()));
    }

    pub fn set_today_steps(&self, steps: u64, done: impl FnOnce(bool) + 'static) {
        self.call("SetTodaySteps", Some((steps,).to_variant()), move |reply| done(reply.is_ok()));
    }

    /// Asks the recorder to reread what the interface changed in the database.
    pub fn reload(&self) {
        self.call("Reload", None, |_| {});
    }
}
