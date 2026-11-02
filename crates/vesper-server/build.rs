//! Sets `VESPER_BUILD`, the build id in `hello` and `/health`. First that is set: the
//! `VESPER_BUILD_ID` variable at build time (the Dockerfile's build arg), `build-id.txt` as
//! `git archive` filled it in, the git commit, else the package version.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=VESPER_BUILD_ID");
    println!("cargo:rerun-if-changed=build-id.txt");
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    // The reflog changes on every commit and checkout, so a new commit reruns this.
    if let Some(log) = git(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        "logs/HEAD",
    ]) {
        println!("cargo:rerun-if-changed={log}");
    }
    let file = std::fs::read_to_string("build-id.txt").unwrap_or_default();
    let file = file.trim();
    let id = std::env::var("VESPER_BUILD_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| (!file.is_empty() && !file.starts_with("$Format")).then(|| file.to_owned()))
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());
    println!("cargo:rustc-env=VESPER_BUILD={id}");
}
