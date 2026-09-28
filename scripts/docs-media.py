#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Record the screenshots and the demo animation of the documentation from
# the real binary: hyprtilt runs against the in-memory Hyprland of demo/ in
# a pseudo-terminal, the pyte terminal emulator follows the screen, and
# Pillow draws it.
#
#   docs/assets/screenshots/<name>.png   screens shown by the guide
#   docs/assets/demo.gif                 the whole session, one caption per key
#
# The demo runs on a copy of demo/, so the repository is never changed.
# Needs pyte and Pillow; "just media" runs it with uv.
#
# Usage: scripts/docs-media.py [--bin target/release/hyprtilt] [--out docs/assets]

import argparse
import fcntl
import os
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

import pyte
from PIL import Image, ImageDraw, ImageFont

COLUMNS, ROWS = 104, 30
FONT_SIZE = 16
PADDING = 18

# Catppuccin Mocha, close to the documentation's dark theme.
BACKGROUND = "#1e1e2e"
FOREGROUND = "#cdd6f4"
PALETTE = {
    "black": "#45475a",
    "red": "#f38ba8",
    "green": "#a6e3a1",
    "brown": "#f9e2af",
    "yellow": "#f9e2af",
    "blue": "#89b4fa",
    "magenta": "#f5c2e7",
    "cyan": "#89dceb",
    "white": "#bac2de",
    "brightblack": "#6c7086",
    "brightred": "#f38ba8",
    "brightgreen": "#a6e3a1",
    "brightbrown": "#f9e2af",
    "brightyellow": "#f9e2af",
    "brightblue": "#89b4fa",
    "brightmagenta": "#f5c2e7",
    "brightcyan": "#89dceb",
    "brightwhite": "#a6adc8",
}
CAPTION_BACKGROUND = "#181825"
CAPTION_KEY = "#89dceb"

# The session: keys, what they do, how long the frame stays in the
# animation (ms), and the screenshot taken after them.
STEPS = [
    ("", "hyprtilt finds the rules you wrote and offers to adopt them", 3200, "adopt"),
    ("y", "adopt them into the managed block", 2200, None),
    ("3", "select DP-1", 1400, None),
    ("[", "lower the refresh rate", 1600, None),
    ("[", "and once more", 1600, "refresh"),
    ("]", "raise it again", 1400, None),
    ("m", "every mode of the monitor", 2200, "modes"),
    ("\x1b", "close the list", 900, None),
    ("1", "select HDMI-A-1, a portrait monitor", 1400, None),
    ("r", "rotate it: the neighbours move with its edge", 2000, None),
    ("R", "and back", 1600, None),
    ("2", "select eDP-1", 1200, None),
    ("kkkk", "move it up by 40 px", 1400, None),
    ("b", "align its bottom edge with the nearest neighbour", 1800, None),
    ("a", "apply live: verified, then counted down", 2600, "countdown"),
    ("y", "keep it", 1800, None),
    ("w", "write the block into the file (already running: no countdown)", 2600, None),
    ("?", "every key", 2600, "help"),
    ("q", "", 600, None),
    ("q", "quit", 1200, None),
]

KEY_NAMES = {"\x1b": "Esc", "": ""}


def font_file(bold):
    name = "DejaVu Sans Mono:bold" if bold else "DejaVu Sans Mono"
    try:
        out = subprocess.run(
            ["fc-match", "-f", "%{file}", name], capture_output=True, text=True, check=True
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        out = ""
    if not out or not Path(out).exists():
        sys.exit(f"docs-media: no font for {name!r}; install DejaVu Sans Mono")
    return out


class Terminal:
    """hyprtilt in a pseudo-terminal, followed by pyte."""

    def __init__(self, binary, workdir):
        self.screen = pyte.Screen(COLUMNS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(workdir / "home"),
            "XDG_CONFIG_HOME": str(workdir / "home" / ".config"),
            "XDG_RUNTIME_DIR": str(workdir / "run"),
            "TERM": "xterm-256color",
            "LANG": "C.UTF-8",
        }
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(workdir)
            # A relative setup path keeps the file name short on screen.
            os.execve(binary, [binary, "--fake-hyprland", "fake.json"], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLUMNS, 0, 0))
        self.settle()

    def settle(self, quiet=0.5, limit=6.0):
        """Read until the screen has not changed for `quiet` seconds.

        The output never stops (every frame hides the cursor), so the
        screen content decides, not the bytes."""
        end = time.monotonic() + limit
        last = time.monotonic()
        shown = self.screen.display
        while time.monotonic() < end:
            ready, _, _ = select.select([self.fd], [], [], 0.05)
            if ready:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                self.stream.feed(data)
                if self.screen.display != shown:
                    shown = self.screen.display
                    last = time.monotonic()
            if time.monotonic() - last >= quiet:
                return

    def press(self, keys):
        for key in keys:
            os.write(self.fd, key.encode())
            # Esc alone must not be read as the start of a sequence.
            time.sleep(0.12 if key == "\x1b" else 0.05)
        self.settle()

    def snapshot(self):
        return [[self.screen.buffer[y][x] for x in range(COLUMNS)] for y in range(ROWS)]

    def close(self):
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        _, status = os.waitpid(self.pid, 0)
        return status


def color(name, default):
    if name == "default":
        return default
    if name in PALETTE:
        return PALETTE[name]
    if len(name) == 6:
        return "#" + name
    return default


class Painter:
    def __init__(self):
        self.regular = ImageFont.truetype(font_file(False), FONT_SIZE)
        self.bold = ImageFont.truetype(font_file(True), FONT_SIZE)
        ascent, descent = self.regular.getmetrics()
        self.cell_w = round(self.regular.getlength("M"))
        self.cell_h = ascent + descent
        self.width = COLUMNS * self.cell_w + 2 * PADDING
        self.term_h = ROWS * self.cell_h + 2 * PADDING

    def screen(self, cells, caption=None):
        caption_h = round(self.cell_h * 2.2) if caption is not None else 0
        image = Image.new("RGB", (self.width, self.term_h + caption_h), BACKGROUND)
        draw = ImageDraw.Draw(image)
        for y, row in enumerate(cells):
            top = PADDING + y * self.cell_h
            for x, cell in enumerate(row):
                fg = color(cell.fg, FOREGROUND)
                bg = color(cell.bg, BACKGROUND)
                if cell.reverse:
                    fg, bg = bg, fg
                left = PADDING + x * self.cell_w
                if bg != BACKGROUND:
                    draw.rectangle(
                        [left, top, left + self.cell_w - 1, top + self.cell_h - 1], fill=bg
                    )
                if cell.data.strip():
                    font = self.bold if cell.bold else self.regular
                    draw.text((left, top), cell.data, font=font, fill=fg)
        if caption is not None:
            self.caption(draw, caption)
        return image

    def caption(self, draw, caption):
        key, text = caption
        top = self.term_h
        draw.rectangle([0, top, self.width, top + round(self.cell_h * 2.2)], fill=CAPTION_BACKGROUND)
        y = top + round(self.cell_h * 0.6)
        x = PADDING
        if key:
            label = f" {key} "
            w = self.bold.getlength(label)
            draw.rounded_rectangle(
                [x, y - 3, x + w, y + self.cell_h + 1], radius=4, fill=CAPTION_KEY
            )
            draw.text((x, y), label, font=self.bold, fill=BACKGROUND)
            x += w + self.cell_w
        draw.text((x, y), text, font=self.regular, fill=FOREGROUND)


def key_label(keys):
    if keys in KEY_NAMES:
        return KEY_NAMES[keys]
    return " ".join(keys) if len(set(keys)) > 1 else keys[0] + (f" x{len(keys)}" if len(keys) > 1 else "")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", default="target/release/hyprtilt")
    parser.add_argument("--out", default="docs/assets")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    binary = str((root / args.bin).resolve())
    out = root / args.out
    shots = out / "screenshots"
    shots.mkdir(parents=True, exist_ok=True)

    painter = Painter()
    frames, durations = [], []
    with tempfile.TemporaryDirectory(prefix="hyprtilt-demo-") as tmp:
        work = Path(tmp)
        for name in ("fake.json", "hypr-user.lua"):
            shutil.copy(root / "demo" / name, work / name)
        (work / "home").mkdir()
        (work / "run").mkdir()
        term = Terminal(binary, work)
        try:
            for keys, text, duration, shot in STEPS:
                if keys:
                    term.press(keys)
                cells = term.snapshot()
                if shot:
                    painter.screen(cells).save(shots / f"{shot}.png", optimize=True)
                    print(f"docs-media: {shots / shot}.png")
                if text:
                    frames.append(painter.screen(cells, (key_label(keys), text)))
                    durations.append(duration)
        finally:
            status = term.close()
    if os.waitstatus_to_exitcode(status) not in (0, -signal.SIGTERM):
        sys.exit(f"docs-media: hyprtilt exited with {os.waitstatus_to_exitcode(status)}")

    palette_frames = [f.quantize(colors=96, method=Image.Quantize.MEDIANCUT) for f in frames]
    gif = out / "demo.gif"
    palette_frames[0].save(
        gif,
        save_all=True,
        append_images=palette_frames[1:],
        duration=durations,
        loop=0,
        optimize=True,
        disposal=1,
    )
    print(f"docs-media: {gif} ({len(frames)} frames)")


if __name__ == "__main__":
    main()
