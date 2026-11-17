# peroxide (app)

The IDE executable. Builds the window and menus, and registers the shell's
commands on the bus (`src/commands.rs`). Menu items call commands; they never
change state directly. License: GPL-3.0-or-later.
