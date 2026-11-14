// A release build on Windows is a GUI app: no console window behind it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    vesper_desktop::ui::run();
}
