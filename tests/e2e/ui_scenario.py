#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Drives the real interface through AT-SPI against the real recorder and a
synthetic sensor. Requires an X server and an accessibility bus.

    xvfb-run -a -s "-screen 0 320x640x24" dbus-run-session -- \
        python3 tests/e2e/ui_scenario.py target/debug
"""
import ctypes
import datetime
import os
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gi

gi.require_version("Atspi", "2.0")
gi.require_version("Gio", "2.0")
from gi.repository import Atspi, Gio, GLib
from recorder_scenario import (FakeSensord, MANAGER_XML, NOTIFY_XML, SENSOR_XML, expect, export, spawn,
                               state, wait_for)


PR_SET_PDEATHSIG = 1
started = []


def launch(*arguments, **options):
    """Starts a process that is stopped when the scenario ends, whatever the outcome."""
    process = subprocess.Popen(*arguments, **options)
    started.append(process)
    return process


def stop_started():
    for process in reversed(started):
        process.terminate()
    for process in started:
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()


def application():
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        child = desktop.get_child_at_index(index)
        if child and "pace" in (child.get_name() or "").lower():
            return child
    return None


def nodes(node, found=None):
    found = [] if found is None else found
    found.append(node)
    for index in range(node.get_child_count()):
        child = node.get_child_at_index(index)
        if child:
            nodes(child, found)
    return found


def by_name(name, role=None):
    return [n for n in nodes(application())
            if n.get_name() == name and (role is None or n.get_role_name() == role)]


def click(name):
    wait_for(lambda: by_name(name, "button"), f"button {name}")
    by_name(name, "button")[0].get_action_iface().do_action(0)
    time.sleep(0.5)


def has_label(text):
    return any(text in (n.get_name() or "") for n in nodes(application()))


def showing(node):
    return node.get_state_set().contains(Atspi.StateType.SHOWING)


def scenario(directory, binaries, fake):
    environment = dict(os.environ, XDG_DATA_HOME=directory, PACE_SENSOR_BUS="session",
                       PACE_SENSOR_SOCKET=os.path.join(directory, "sensord.sock"),
                       GTK_A11Y="atspi", GDK_BACKEND="x11", GSK_RENDERER="cairo", LANGUAGE="C", NO_AT_BRIDGE="0")
    today = datetime.date.today()
    data = os.path.join(directory, "pace-beads")
    os.makedirs(data)

    interface = launch([os.path.join(binaries, "pace-beads")], env=environment)
    wait_for(lambda: application() is not None and has_label("Goal"), "window", timeout=15)
    notice = by_name("The recorder is not running. Steps are not being counted.", "label")
    expect(bool(notice and showing(notice[0])), True, "missing recorder is announced")
    interface.terminate()
    interface.wait()

    recorder = spawn(os.path.join(binaries, "pace-beads-recorder"), environment)
    started.append(recorder)
    wait_for(lambda: state()[2], "sensor")
    connection = sqlite3.connect(os.path.join(data, "history.sqlite"))
    with connection:
        for days in range(1, 40):
            connection.execute("INSERT INTO days VALUES (?, ?)",
                               ((today - datetime.timedelta(days=days)).isoformat(), 12000))
    recorder_interface = launch([os.path.join(binaries, "pace-beads")], env=environment)
    wait_for(lambda: application() is not None and has_label("Goal"), "window", timeout=15)

    hints = subprocess.run(["xprop", "-name", "Pace Beads", "WM_NORMAL_HINTS"],
                           capture_output=True, text=True).stdout
    minimum = next(line for line in hints.splitlines() if "minimum size" in line)
    print(minimum.strip())
    expect(int(minimum.split(":")[1].split()[0]) <= 360, True, "window can be as narrow as a phone")

    fake.emit(150)
    wait_for(lambda: by_name("50", "label"), "live total in the window")
    print("ok - live steps reach the window")
    expect(has_label("39 days"), True, "streak from history is shown")

    click("Pause")
    click("30 Minutes")
    wait_for(lambda: state()[1].startswith("until:"), "timed pause in the recorder")
    print("ok - pause dialog reaches the recorder")
    wait_for(lambda: by_name("Resume", "button"), "resume button")
    fake.emit(170)
    time.sleep(0.3)
    expect(state()[0], 50, "paused steps are not counted")
    click("Resume")
    wait_for(lambda: state()[1] == "running", "resume")
    print("ok - resume works from the window")

    click("Edit Steps")
    click("Save")
    expect(state()[0], 50, "unchanged edit keeps the total")

    subprocess.run(["gapplication", "action", "io.github.x_vlad_x.PaceBeads", "preferences"],
                   check=True, env=environment, timeout=10)
    wait_for(lambda: has_label("Daily Goal") and has_label("Profile"), "preferences")
    print("ok - preferences open")

    recorder_interface.terminate()
    recorder_interface.wait()
    recorder.terminate()
    recorder.wait()


def stop_with_parent():
    """Keeps a crashed scenario from leaving its helper behind."""
    ctypes.CDLL(None).prctl(PR_SET_PDEATHSIG, signal.SIGTERM)


def serve(context, loop, fake, ready):
    """Runs the fake services on their own context: libatspi is not thread safe and
    stays on the main thread, whose default context nothing else iterates."""
    context.push_thread_default()

    def sensor_ready(connection, _name):
        export(connection, MANAGER_XML, "/SensorManager", fake.handle)
        export(connection, SENSOR_XML, "/SensorManager/stepcountersensor", fake.handle)

    def notify_ready(connection, _name):
        export(connection, NOTIFY_XML, "/org/freedesktop/Notifications", fake.handle)

    Gio.bus_own_name(Gio.BusType.SESSION, "com.nokia.SensorService", 0, sensor_ready, None, None)
    Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.Notifications", 0, notify_ready, None, None)
    ready.set()
    loop.run()
    context.pop_thread_default()


def main():
    binaries = os.path.abspath(sys.argv[1])
    directory = tempfile.mkdtemp(prefix="pace-ui-")
    # A private session has no service manager to activate the registry.
    registry = subprocess.Popen(["/usr/lib/at-spi2-registryd"], stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, preexec_fn=stop_with_parent)
    started.append(registry)
    time.sleep(1)
    Atspi.init()
    fake = FakeSensord(os.path.join(directory, "sensord.sock"))
    context = GLib.MainContext.new()
    loop = GLib.MainLoop.new(context, False)
    ready = threading.Event()
    threading.Thread(target=serve, args=(context, loop, fake, ready), daemon=True).start()
    ready.wait()
    try:
        time.sleep(0.5)
        scenario(directory, binaries, fake)
        outcome = 0
    except Exception as error:  # reported as the scenario's result
        print(f"FAILED: {error}")
        outcome = 1
    finally:
        stop_started()
        loop.quit()
    sys.exit(outcome)


if __name__ == "__main__":
    main()
