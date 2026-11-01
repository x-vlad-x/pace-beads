# Screenshots

These images show synthetic data on a real FLX1s. The history comes from a fixed seed in `tests/e2e/screenshots.py`, and no real walking records are involved.

To retake them, copy `tests/e2e/*.py` to the phone, unlock it, and for each language and theme run:

```sh
runtime=$(mktemp -d)
WAYLAND_DISPLAY=/run/user/$(id -u)/wayland-0 XDG_RUNTIME_DIR=$runtime \
    dbus-run-session -- python3 screenshots.py <binary dir> <output dir> en light today history achievements preferences
```

The script runs on a private bus with a temporary data directory, leaves the installed recorder alone and exits with status 2 if a widget does not fit the window. Then, on any machine with Pillow:

```sh
python3 tests/e2e/prepare_screenshots.py <output dir> docs/screenshots
```
