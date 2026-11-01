// SPDX-License-Identifier: GPL-3.0-only
//! Client for the SensorFW step counter: D-Bus control plus a framed Unix socket.
//!
//! SensorFW releases the session when either the socket or the D-Bus
//! connection closes, so both stay open for the life of the session.

use glib::variant::ToVariant;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const SERVICE: &str = "com.nokia.SensorService";
const MANAGER_PATH: &str = "/SensorManager";
const MANAGER_INTERFACE: &str = "local.SensorManager";
const SENSOR_ID: &str = "stepcountersensor";
const SENSOR_PATH: &str = "/SensorManager/stepcountersensor";
const SENSOR_INTERFACE: &str = "local.StepCounterSensor";
const DEFAULT_SOCKET: &str = "/run/sensord.sock";
const CALL_TIMEOUT_MS: i32 = 5000;

/// Size of one `TimedUnsigned` record: u64 time, u32 value, padding.
const RECORD_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub timestamp_us: u64,
    pub value: u32,
}

/// Incremental decoder for `u32 count` + `count` records.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Reading> {
        self.buffer.extend_from_slice(bytes);
        let mut readings = Vec::new();
        let mut offset = 0;
        while let Some(head) = self.buffer.get(offset..offset + 4) {
            let count = u32::from_ne_bytes([head[0], head[1], head[2], head[3]]) as usize;
            let end = offset + 4 + count * RECORD_SIZE;
            let Some(body) = self.buffer.get(offset + 4..end) else {
                break;
            };
            readings.extend(body.chunks_exact(RECORD_SIZE).map(|record| Reading {
                timestamp_us: u64::from_ne_bytes(record[0..8].try_into().unwrap_or_default()),
                value: u32::from_ne_bytes(record[8..12].try_into().unwrap_or_default()),
            }));
            offset = end;
        }
        self.buffer.drain(..offset);
        readings
    }
}

pub struct Session {
    pub stream: UnixStream,
    connection: gio::DBusConnection,
    session_id: i32,
    decoder: FrameDecoder,
}

fn bus_type() -> gio::BusType {
    match std::env::var("PACE_SENSOR_BUS").as_deref() {
        Ok("session") => gio::BusType::Session,
        _ => gio::BusType::System,
    }
}

fn call(
    connection: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    arguments: &glib::Variant,
    reply: Option<&glib::VariantTy>,
) -> Result<glib::Variant, glib::Error> {
    connection.call_sync(
        Some(SERVICE),
        path,
        interface,
        method,
        Some(arguments),
        reply,
        gio::DBusCallFlags::NONE,
        CALL_TIMEOUT_MS,
        gio::Cancellable::NONE,
    )
}

fn sensor_call(
    connection: &gio::DBusConnection,
    method: &str,
    arguments: &glib::Variant,
) -> Result<glib::Variant, glib::Error> {
    call(connection, SENSOR_PATH, SENSOR_INTERFACE, method, arguments, None)
}

fn failure(message: String) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, &message)
}

impl Session {
    /// Opens a session and starts delivery.
    pub fn open() -> Result<Self, glib::Error> {
        let connection = gio::bus_get_sync(bus_type(), gio::Cancellable::NONE)?;
        let pid = i64::from(std::process::id());

        let loaded = call(
            &connection,
            MANAGER_PATH,
            MANAGER_INTERFACE,
            "loadPlugin",
            &(SENSOR_ID,).to_variant(),
            Some(glib::VariantTy::new("(b)").expect("valid type")),
        )?;
        if loaded.get::<(bool,)>().is_none_or(|(ok,)| !ok) {
            return Err(failure("the step counter plugin did not load".into()));
        }

        let requested = call(
            &connection,
            MANAGER_PATH,
            MANAGER_INTERFACE,
            "requestSensor",
            &(SENSOR_ID, pid).to_variant(),
            Some(glib::VariantTy::new("(i)").expect("valid type")),
        )?;
        let session_id = requested.get::<(i32,)>().map_or(-1, |(id,)| id);
        if session_id < 0 {
            return Err(failure(format!("the sensor request returned {session_id}")));
        }

        let socket = std::env::var("PACE_SENSOR_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET.into());
        let attach = || -> std::io::Result<UnixStream> {
            let mut stream = UnixStream::connect(&socket)?;
            stream.write_all(&session_id.to_ne_bytes())?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut tag = [0u8; 1];
            stream.read_exact(&mut tag)?;
            stream.set_read_timeout(None)?;
            stream.set_nonblocking(true)?;
            Ok(stream)
        };
        let stream = attach().map_err(|error| failure(format!("sensor socket: {error}")))?;

        sensor_call(&connection, "setStandbyOverride", &(session_id, true).to_variant())?;
        sensor_call(&connection, "start", &(session_id,).to_variant())?;
        Ok(Self { stream, connection, session_id, decoder: FrameDecoder::default() })
    }

    /// Current counter value, when the daemon knows a nonzero one.
    ///
    /// Zero is what SensorFW reports before its first event after a restart,
    /// so it cannot be told apart from a real position.
    pub fn current_value(&self) -> Option<u32> {
        let reply = sensor_call(&self.connection, "steps", &().to_variant()).ok()?;
        let ((_timestamp, value),) = reply.get::<((u64, u32),)>()?;
        (value > 0).then_some(value)
    }

    /// Drains the socket. `None` means the connection is gone.
    pub fn read_available(&mut self) -> Option<Vec<Reading>> {
        let mut readings = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return None,
                Ok(size) => readings.extend(self.decoder.push(&chunk[..size])),
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Some(readings),
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
    }

    pub fn session_id(&self) -> i32 {
        self.session_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(records: &[(u64, u32)]) -> Vec<u8> {
        let mut bytes = (records.len() as u32).to_ne_bytes().to_vec();
        for (timestamp, value) in records {
            bytes.extend(timestamp.to_ne_bytes());
            bytes.extend(value.to_ne_bytes());
            bytes.extend(0u32.to_ne_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_one_frame() {
        let readings = FrameDecoder::default().push(&frame(&[(7, 42)]));
        assert_eq!(readings, [Reading { timestamp_us: 7, value: 42 }]);
    }

    #[test]
    fn decodes_several_records_and_frames() {
        let mut bytes = frame(&[(1, 10), (2, 11)]);
        bytes.extend(frame(&[(3, 12)]));
        assert_eq!(FrameDecoder::default().push(&bytes).len(), 3);
    }

    #[test]
    fn keeps_partial_frames_until_complete() {
        let bytes = frame(&[(9, 99)]);
        let mut decoder = FrameDecoder::default();
        for byte in &bytes[..bytes.len() - 1] {
            assert!(decoder.push(std::slice::from_ref(byte)).is_empty());
        }
        assert_eq!(decoder.push(&bytes[bytes.len() - 1..]).len(), 1);
        assert!(decoder.buffer.is_empty());
    }
}
