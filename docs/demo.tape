# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# The demo GIF of the terminal interface, recorded with vhs against the
# in-memory Hyprland of demo/. Run "just demo" (in the build image: "just
# podman demo"); it writes docs/assets/demo.gif.

Output docs/assets/demo.gif

Require bash

Set Shell "bash"
Set FontSize 15
Set Width 1280
Set Height 640
Set Padding 16
Set TypingSpeed 80ms
Set Theme "Catppuccin Mocha"

# A private copy of the demo, so the recording never changes demo/.
Hide
Type "demo=$(mktemp -d) && cp -r demo/. $demo && export PATH=$PWD/target/release:$PATH && clear"
Enter
Show

Type "hyprtilt --fake-hyprland $demo/fake.json"
Sleep 500ms
Enter
Sleep 2s

# Adopt the three hand-written rules into the managed block.
Type "y"
Sleep 2s

# DP-1: step the refresh rate down and back up.
Type "3"
Sleep 800ms
Type "["
Sleep 1.2s
Type "]"
Sleep 1.2s

# HDMI-A-1: rotate to landscape; its neighbours move along.
Type "1"
Sleep 800ms
Type "r"
Sleep 1.5s
Type "R"
Sleep 1.5s

# eDP-1: move it up and align its bottom edge with the neighbour.
Type "2"
Sleep 600ms
Type "kkkk"
Sleep 1s
Type "b"
Sleep 1.5s

# Apply live, confirm, then write into the file and confirm.
Type "a"
Sleep 2.5s
Type "y"
Sleep 1.5s
Type "w"
Sleep 2s
Type "y"
Sleep 2s

Type "q"
Sleep 500ms

Hide
Type "rm -rf $demo"
Enter
