# perox-terminal

The integrated terminal. alacritty_terminal (Apache-2.0) does the emulation and the PTY
(macOS, Linux, Windows ConPTY); this crate turns egui input into terminal bytes (xterm
conventions) and draws the cell grid with the 256-colour palette. The shell gets the
pinned-toolchain environment, so `cargo` in it is the Toolchain Set's. License: GPL-3.0-or-later.
