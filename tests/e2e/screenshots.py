#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Takes the README screenshots on a phone from synthetic data, and reports
any widget that does not fit the window in that language.

Everything runs on a private session bus with a temporary data directory and
a fake sensor, so a recorder that is already running and its history are not
touched. Requires grim, wtype, an unlocked Wayland session and an AT-SPI registry.

Run on the phone from a directory with the files of tests/e2e. The private
session needs its own runtime directory: services it activates, gvfs among
them, would otherwise take over mounts of the real session and break them.
    runtime=$(mktemp -d)
    WAYLAND_DISPLAY=/run/user/$(id -u)/wayland-0 XDG_RUNTIME_DIR=$runtime \\
        dbus-run-session -- python3 screenshots.py <binary dir> <output dir> <language> <theme> <page>...

language: en, de or ru
theme:    light or dark
page:     today, history, achievements, preferences, about, pause or edit
"""
import datetime
import os
import random
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib

from recorder_scenario import FakeSensord, spawn, state, wait_for
from ui_scenario import application, nodes, serve, stop_with_parent

APP_ID = "io.github.x_vlad_x.PaceBeads"
REGISTRY = "/usr/libexec/at-spi2-registryd"
THEMES = {"light": "1", "dark": "2"}
HISTORY_DAYS = 84
TODAY_STEPS = 7842
GOAL = 10000
MAXIMUM_EXTENT = 100000

LANGUAGES = {
    "en": {"locale": "en_US.UTF-8", "today": "Today", "history": "History", "achievements": "Achievements",
           "pause": "Pause", "edit": "Edit Steps"},
    "de": {"locale": "de_DE.UTF-8", "today": "Heute", "history": "Verlauf", "achievements": "Erfolge",
           "pause": "Pause", "edit": "Bearbeiten"},
    "ru": {"locale": "ru_RU.UTF-8", "today": "Сегодня", "history": "История", "achievements": "Достижения",
           "pause": "Пауза", "edit": "Изменить"},
}


def fill_history(database, theme):
    """A plausible walking pattern. Seeded, so every run draws the same days."""
    today = datetime.date.today()
    generator = random.Random(2026)
    connection = sqlite3.connect(database)
    with connection:
        connection.execute("DELETE FROM days")
        connection.execute("INSERT INTO days VALUES (?, ?)", (today.isoformat(), TODAY_STEPS))
        for age in range(1, HISTORY_DAYS):
            if age <= 5:
                steps = GOAL + generator.randint(100, 2500)
            elif generator.random() < 0.18:
                steps = generator.randint(2100, 8800)
            else:
                steps = generator.randint(6200, 14800)
            day = today - datetime.timedelta(days=age)
            connection.execute("INSERT INTO days VALUES (?, ?)", (day.isoformat(), steps))
        first = today - datetime.timedelta(days=HISTORY_DAYS)
        connection.execute("INSERT OR REPLACE INTO goal_changes VALUES (?, ?)", (first.isoformat(), GOAL))
        for name, value in (("theme", theme), ("date_format", "1")):
            connection.execute("INSERT OR REPLACE INTO settings VALUES (?, ?)", (name, value))
    connection.close()


def press_named(name):
    """Activates a button or view switcher entry with this accessible name."""
    def find():
        return [node for node in nodes(application())
                if node.get_name() == name and node.get_action_iface() is not None
                and node.get_role_name() in ("button", "page tab")]
    wait_for(find, f"control {name}")
    find()[0].get_action_iface().do_action(0)
    time.sleep(1.2)


def report_overflow(page):
    """Prints shown widgets that extend past the window's right edge."""
    window = next(node for node in nodes(application()) if node.get_role_name() == "frame"
                  or node.get_role_name() == "window")
    width = window.get_component_iface().get_size().x
    problems = 0
    for node in nodes(application()):
        component = node.get_component_iface()
        states = node.get_state_set()
        if component is None or not states.contains(Atspi.StateType.SHOWING):
            continue
        box = component.get_extents(Atspi.CoordType.WINDOW)
        # Stack pages that were never mapped report uninitialized extents.
        if width + 1 < box.x + box.width < MAXIMUM_EXTENT and node.get_name():
            problems += 1
            print(f"OVERFLOW {page}: {node.get_role_name()} {node.get_name()!r} ends at {box.x + box.width} of {width}")
    print(f"layout {page}: window width {width}, {problems} overflowing")
    return problems


def run(binaries, output, language, theme, pages, directory):
    words = LANGUAGES[language]
    locale = words["locale"]
    environment = dict(os.environ, XDG_DATA_HOME=directory, PACE_SENSOR_BUS="session",
                       PACE_SENSOR_SOCKET=os.path.join(directory, "sensord.sock"),
                       LANGUAGE=language, LC_ALL=locale, LANG=locale, GTK_A11Y="atspi", NO_AT_BRIDGE="0",
                       GDK_BACKEND="wayland", GIO_USE_VFS="local")
    database = os.path.join(directory, "pace-beads/history.sqlite")

    recorder = spawn(os.path.join(binaries, "pace-beads-recorder"), environment)
    wait_for(lambda: state()[2], "sensor")
    recorder.terminate()
    recorder.wait()
    fill_history(database, THEMES[theme])
    recorder = spawn(os.path.join(binaries, "pace-beads-recorder"), environment)
    wait_for(lambda: state()[2], "sensor")

    interface = subprocess.Popen([os.path.join(binaries, "pace-beads")], env=environment)
    os.makedirs(output, exist_ok=True)
    problems = 0
    try:
        wait_for(lambda: application() is not None, "window", timeout=20)
        time.sleep(3)
        for page in pages:
            if page in ("preferences", "about"):
                subprocess.run(["gapplication", "action", APP_ID, page], env=environment, check=True, timeout=10)
                time.sleep(1.5)
            elif page in ("pause", "edit"):
                press_named(words["today"])
                press_named(words[page])
            else:
                press_named(words[page])
            problems += report_overflow(page)
            target = os.path.join(output, f"{page}-{theme}-{language}.png")
            subprocess.run(["grim", target], check=True)
            print(target)
            if page in ("preferences", "about", "pause", "edit"):
                subprocess.run(["wtype", "-k", "Escape"], check=True)
                time.sleep(1)
    finally:
        interface.terminate()
        interface.wait()
        recorder.terminate()
        recorder.wait()
    return problems


def main():
    binaries, output, language, theme, *pages = sys.argv[1:]
    directory = tempfile.mkdtemp(prefix="pace-screenshots-")
    registry = subprocess.Popen([REGISTRY], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                preexec_fn=stop_with_parent)
    time.sleep(1)
    Atspi.init()
    fake = FakeSensord(os.path.join(directory, "sensord.sock"))
    context = GLib.MainContext.new()
    loop = GLib.MainLoop.new(context, False)
    ready = threading.Event()
    threading.Thread(target=serve, args=(context, loop, fake, ready), daemon=True).start()
    ready.wait()
    outcome = 1
    try:
        time.sleep(0.5)
        problems = run(os.path.abspath(binaries), os.path.abspath(output), language, theme, pages, directory)
        outcome = 2 if problems else 0
    finally:
        registry.terminate()
        loop.quit()
        shutil.rmtree(directory, ignore_errors=True)
    sys.exit(outcome)


if __name__ == "__main__":
    main()
