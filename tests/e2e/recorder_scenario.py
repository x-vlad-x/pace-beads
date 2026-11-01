#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""End-to-end recorder scenario against a synthetic SensorFW service.

Run inside a private bus:
    dbus-run-session -- python3 tests/e2e/recorder_scenario.py target/debug/pace-beads-recorder
"""
import datetime
import os
import signal
import socket
import sqlite3
import struct
import subprocess
import sys
import tempfile
import threading
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

MANAGER_XML = """<node><interface name="local.SensorManager">
<method name="loadPlugin"><arg type="s" direction="in"/><arg type="b" direction="out"/></method>
<method name="requestSensor"><arg type="s" direction="in"/><arg type="x" direction="in"/><arg type="i" direction="out"/></method>
</interface></node>"""
SENSOR_XML = """<node><interface name="local.StepCounterSensor">
<method name="setStandbyOverride"><arg type="i" direction="in"/><arg type="b" direction="in"/><arg type="b" direction="out"/></method>
<method name="start"><arg type="i" direction="in"/></method>
<method name="steps"><arg type="(tu)" direction="out"/></method>
</interface></node>"""
NOTIFY_XML = """<node><interface name="org.freedesktop.Notifications">
<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
<arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/>
<arg type="i" direction="in"/><arg type="u" direction="out"/></method>
</interface></node>"""


class FakeSensord:
    def __init__(self, path):
        self.value = 100
        self.next_session = 1
        self.sessions = {}
        self.standby = []
        self.notifications = []
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(path)
        self.listener.listen(5)
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        while True:
            client, _ = self.listener.accept()
            session = struct.unpack("=i", client.recv(4))[0]
            self.sessions[session] = client
            client.sendall(b"\n")

    def emit(self, value):
        self.value = value
        frame = struct.pack("=IQII", 1, 1234, value, 0)
        for client in self.sessions.values():
            client.sendall(frame)

    def drop_clients(self):
        for client in self.sessions.values():
            client.close()
        self.sessions.clear()

    def handle(self, _c, _s, _p, _i, method, args, invocation):
        if method == "loadPlugin":
            invocation.return_value(GLib.Variant("(b)", (True,)))
        elif method == "requestSensor":
            self.next_session += 1
            invocation.return_value(GLib.Variant("(i)", (self.next_session,)))
        elif method == "setStandbyOverride":
            self.standby.append(tuple(args))
            invocation.return_value(GLib.Variant("(b)", (True,)))
        elif method == "start":
            invocation.return_value(None)
        elif method == "steps":
            invocation.return_value(GLib.Variant("((tu))", ((0, self.value),)))
        elif method == "Notify":
            self.notifications.append(args[3])
            invocation.return_value(GLib.Variant("(u)", (1,)))


def export(connection, xml, path, handler):
    info = Gio.DBusNodeInfo.new_for_xml(xml)
    connection.register_object(path, info.interfaces[0], handler, None, None)


def recorder_call(method, *arguments):
    command = ["gdbus", "call", "--session", "--dest", "io.github.x_vlad_x.PaceBeads.Recorder",
               "--object-path", "/io/github/x_vlad_x/PaceBeads/Recorder",
               "--method", "io.github.x_vlad_x.PaceBeads.Recorder." + method, *arguments]
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout.strip()


def state():
    # Output looks like: (uint64 20, 'running', true)
    text = recorder_call("GetState").strip("()").split(", ")
    return int(text[0].split()[-1]), text[1].strip("'"), text[2] == "true"


def wait_for(condition, what, timeout=8):
    end = time.time() + timeout
    while time.time() < end:
        if condition():
            return
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


def expect(actual, wanted, what):
    if actual != wanted:
        raise AssertionError(f"{what}: expected {wanted!r}, got {actual!r}")
    print(f"ok - {what}")


spawned = []


def spawn(binary, environment):
    process = subprocess.Popen([binary], env=environment)
    spawned.append(process)
    wait_for(lambda: subprocess.run(
        ["gdbus", "call", "--session", "--dest", "org.freedesktop.DBus", "--object-path",
         "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.NameHasOwner",
         "io.github.x_vlad_x.PaceBeads.Recorder"],
        capture_output=True, text=True).stdout.startswith("(true"), "bus name")
    return process


def scenario(binary, fake, directory):
    environment = dict(os.environ, XDG_DATA_HOME=directory, PACE_SENSOR_BUS="session",
                       PACE_SENSOR_SOCKET=os.path.join(directory, "sensord.sock"))
    database = os.path.join(directory, "pace-beads/history.sqlite")

    process = spawn(binary, environment)
    wait_for(lambda: state()[2], "sensor connection")
    expect(state()[0], 0, "the first reading only sets the base")
    expect(fake.standby[-1][1], True, "standby override requested")

    fake.emit(110)
    fake.emit(120)
    wait_for(lambda: state()[0] == 20, "two samples")
    print("ok - samples are counted")

    process.kill()
    process.wait()
    fake.drop_clients()
    # Steps the sensor counted before the crash but the recorder had not stored.
    # The real sensor does not advance while no client holds a session.
    fake.value = 150
    process = spawn(binary, environment)
    wait_for(lambda: state()[0] == 50, "recovery after SIGKILL")
    print("ok - SIGKILL loses no steps the sensor reported afterwards")

    recorder_call("Pause", "0")
    expect(state()[1], "indefinite", "pause is reported")
    fake.emit(200)
    time.sleep(0.3)
    expect(state()[0], 50, "paused steps are excluded")
    recorder_call("Resume")
    fake.emit(205)
    wait_for(lambda: state()[0] == 55, "steps after resuming")
    print("ok - counting continues after resuming")

    with sqlite3.connect(database) as connection:
        connection.execute("INSERT INTO goal_changes VALUES ('2000-01-01', 1000)")
    recorder_call("Reload")
    recorder_call("SetTodaySteps", "uint64 999")
    fake.emit(206)
    wait_for(lambda: state()[0] == 1000, "manual total plus one step")
    print("ok - manual edit keeps later steps")
    wait_for(lambda: "Daily goal reached" in fake.notifications, "goal notification")
    print("ok - goal notification sent")
    fake.emit(207)
    wait_for(lambda: state()[0] == 1001, "next step")
    expect(fake.notifications.count("Daily goal reached"), 1, "goal notification sent once")

    fake.drop_clients()
    wait_for(lambda: state()[2] is False or len(fake.sessions) > 0, "loss noticed")
    wait_for(lambda: state()[2] and len(fake.sessions) > 0, "reconnection")
    fake.emit(210)
    wait_for(lambda: state()[0] == 1004, "steps after reconnection")
    print("ok - recovers after losing the sensor service")

    process.send_signal(signal.SIGTERM)
    expect(process.wait(timeout=5), 0, "clean exit on SIGTERM")
    today = datetime.date.today().isoformat()
    with sqlite3.connect(database) as connection:
        steps = connection.execute("SELECT steps FROM days WHERE date = ?", (today,)).fetchone()[0]
        value = connection.execute("SELECT value FROM counter").fetchone()[0]
    expect((steps, value), (1004, 210), "totals and base reach the database")


def main():
    binary = os.path.abspath(sys.argv[1])
    directory = tempfile.mkdtemp(prefix="pace-e2e-")
    fake = FakeSensord(os.path.join(directory, "sensord.sock"))
    loop = GLib.MainLoop()
    outcome = []

    def bus_ready(connection, _name):
        export(connection, MANAGER_XML, "/SensorManager", fake.handle)
        export(connection, SENSOR_XML, "/SensorManager/stepcountersensor", fake.handle)

    def notifications_ready(connection, _name):
        export(connection, NOTIFY_XML, "/org/freedesktop/Notifications", fake.handle)

    Gio.bus_own_name(Gio.BusType.SESSION, "com.nokia.SensorService", 0, bus_ready, None, None)
    Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.Notifications", 0, notifications_ready, None, None)

    def run():
        try:
            time.sleep(0.3)
            scenario(binary, fake, directory)
            outcome.append(True)
        except Exception as error:  # reported as the scenario's result
            print(f"FAILED: {error}")
            outcome.append(False)
        finally:
            for process in spawned:
                if process.poll() is None:
                    process.kill()
            GLib.idle_add(loop.quit)

    threading.Thread(target=run, daemon=True).start()
    loop.run()
    sys.exit(0 if outcome and outcome[0] else 1)


if __name__ == "__main__":
    main()
