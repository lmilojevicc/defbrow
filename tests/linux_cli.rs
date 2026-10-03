#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

// Only child processes receive these paths. Even `set` uses fixture tools and
// state files, never the desktop session's real xdg-utils or configuration.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "defbrow-linux-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { root };
        for directory in ["home", "data/applications", "system", "bin"] {
            fs::create_dir_all(fixture.root.join(directory)).unwrap();
        }
        fs::write(
            fixture.root.join("data/applications/unfamiliar.desktop"),
            "[Desktop Entry]\nType=Application\nName=Unfamiliar Handler\nExec=never-run\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n",
        )
        .unwrap();
        for scheme in ["http", "https"] {
            fs::write(fixture.root.join(scheme), "old.desktop\n").unwrap();
        }
        fixture.tool(
            "xdg-mime",
            r##"#!/bin/sh
printf 'mime:%s\n' "$*" >> "$FIXTURE/log"
case "$*" in
  'query default x-scheme-handler/http') read -r value < "$FIXTURE/http" ;;
  'query default x-scheme-handler/https') read -r value < "$FIXTURE/https" ;;
  *) exit 90 ;;
esac
printf '%s\n' "$value"
"##,
        );
        fixture.tool(
            "xdg-settings",
            r##"#!/bin/sh
printf 'settings:%s\n' "$*" >> "$FIXTURE/log"
case "$*" in
  'set default-web-browser unfamiliar.desktop')
    printf '%s\n' "$3" > "$FIXTURE/http"
    if [ "$PARTIAL" != 1 ]; then printf '%s\n' "$3" > "$FIXTURE/https"; fi ;;
  'check default-web-browser unfamiliar.desktop') printf 'yes\n' ;;
  *) exit 91 ;;
esac
"##,
        );
        fixture
    }

    fn tool(&self, name: &str, content: &str) {
        let path = self.root.join("bin").join(name);
        fs::write(&path, content).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, args: &[&str], partial: bool) -> Output {
        Command::new(env!("CARGO_BIN_EXE_defbrow"))
            .args(args)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_DATA_DIRS", self.root.join("system"))
            .env("PATH", self.root.join("bin"))
            .env("LC_ALL", "C")
            .env("FIXTURE", &self.root)
            .env("PARTIAL", if partial { "1" } else { "0" })
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn integrated_list_current_and_invalid_selection_are_read_only() {
    let fixture = Fixture::new();
    let list = fixture.run(&["list"], false);
    assert!(list.status.success(), "{:?}", list);
    assert!(String::from_utf8(list.stdout)
        .unwrap()
        .contains("Unfamiliar Handler\tunfamiliar.desktop\t"));
    let current = fixture.run(&["current"], false);
    assert!(current.status.success());
    assert_eq!(
        String::from_utf8(current.stdout).unwrap(),
        "HTTP: old.desktop\nHTTPS: old.desktop\n"
    );
    let invalid = fixture.run(&["set", "missing.desktop"], false);
    assert!(!invalid.status.success());
    assert!(!fs::read_to_string(fixture.root.join("log"))
        .unwrap()
        .contains("settings:"));
}

#[test]
fn integrated_set_verifies_both_schemes_with_fixture_tools() {
    let fixture = Fixture::new();
    let result = fixture.run(&["set", "Unfamiliar Handler"], false);
    assert!(result.status.success(), "{:?}", result);
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("Default browser: Unfamiliar Handler"));
    let log = fs::read_to_string(fixture.root.join("log")).unwrap();
    assert!(log.starts_with("settings:set default-web-browser unfamiliar.desktop\nsettings:check default-web-browser unfamiliar.desktop\n"));
    for scheme in ["http", "https"] {
        assert_eq!(
            fs::read_to_string(fixture.root.join(scheme)).unwrap(),
            "unfamiliar.desktop\n"
        );
        assert!(log.contains(&format!("mime:query default x-scheme-handler/{scheme}\n")));
    }
}

#[test]
fn integrated_rejection_sanitizes_filename_and_tool_diagnostics() {
    let fixture = Fixture::new();
    fs::rename(
        fixture.root.join("data/applications/unfamiliar.desktop"),
        fixture
            .root
            .join("data/applications/unfamiliar\u{1b}[31m.desktop"),
    )
    .unwrap();
    fixture.tool(
        "xdg-settings",
        r##"#!/bin/sh
printf 'rejected\033[31m payload\r\t\302\233 hidden\n' >&2
exit 4
"##,
    );

    let result = fixture.run(&["set", "Unfamiliar Handler"], false);
    assert_eq!(result.status.code(), Some(1));
    assert!(!String::from_utf8(result.stdout)
        .unwrap()
        .contains("Default browser:"));
    let error = String::from_utf8(result.stderr).unwrap();
    let diagnostic = error.strip_suffix('\n').unwrap();
    assert!(diagnostic.chars().all(|c| !c.is_control()), "{error:?}");
    assert!(diagnostic.contains("Could not verify unfamiliar [31m.desktop"));
    assert!(diagnostic.contains("set failed: xdg-settings set failed"));
    assert!(diagnostic.contains("rejected [31m payload    hidden"));
    assert!(diagnostic.contains("HTTP=\"old.desktop\", HTTPS=\"old.desktop\""));
    for scheme in ["http", "https"] {
        assert_eq!(
            fs::read_to_string(fixture.root.join(scheme)).unwrap(),
            "old.desktop\n"
        );
    }
}

#[test]
fn integrated_partial_set_returns_error_without_success_output() {
    let fixture = Fixture::new();
    let result = fixture.run(&["set", "unfamiliar.desktop"], true);
    assert!(!result.status.success());
    assert!(!String::from_utf8(result.stdout)
        .unwrap()
        .contains("Default browser:"));
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(error.contains("HTTP=\"unfamiliar.desktop\", HTTPS=\"old.desktop\""));
    assert!(error.contains("partially applied"));
    assert_eq!(
        fs::read_to_string(fixture.root.join("https")).unwrap(),
        "old.desktop\n"
    );
}
