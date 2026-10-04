use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::{Duration, Instant},
};

#[cfg(unix)]
#[path = "support/pty.rs"]
mod pty;

use anyhow::{bail, Result};
use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use defbrow::{
    backend::{Backend, Browser, Cancelled, CurrentDefaults},
    execute, resolve_browser,
    ui::{Picker, PickerAction},
    Cli, Command,
};

fn browser(id: &str, name: &str) -> Browser {
    Browser {
        id: id.into(),
        name: name.into(),
        detail: "fixture".into(),
    }
}

#[derive(Default)]
struct FakeBackend {
    browsers: Vec<Browser>,
    current: RefCell<CurrentDefaults>,
    sets: Cell<usize>,
    partial: bool,
    fail: bool,
    fail_once: Cell<bool>,
    readback_fails: bool,
    refresh_fails: bool,
    refreshed_browsers: Option<Vec<Browser>>,
    /// The PTY fixture requires the picker's screen to stay active for the setter.
    expect_picker_screen: bool,
    /// When set, the setter blocks like a native setter awaiting OS approval.
    block_until_released: Option<PathBuf>,
    /// When set, the setter reports a stopped wait instead of changing anything,
    /// carrying the given `Cancelled::requested` value.
    cancel: Option<bool>,
}

impl Backend for FakeBackend {
    fn browsers(&self) -> Result<Vec<Browser>> {
        if self.refresh_fails && self.sets.get() > 0 {
            bail!("Fixture refresh\nfailed");
        }
        Ok(if self.sets.get() > 0 {
            self.refreshed_browsers
                .as_ref()
                .unwrap_or(&self.browsers)
                .clone()
        } else {
            self.browsers.clone()
        })
    }
    fn current(&self) -> Result<CurrentDefaults> {
        if self.readback_fails && self.sets.get() > 0 {
            bail!("Fixture readback\u{1b}failed");
        }
        Ok(self.current.borrow().clone())
    }
    fn set_default(&self, browser: &Browser, interrupted: &dyn Fn() -> bool) -> Result<()> {
        self.sets.set(self.sets.get() + 1);
        let blocked_until = self.block_until_released.as_deref();
        if blocked_until.is_some() {
            // Announced before the checks below, so a test can wait for the setter to be
            // entered even once the picker screen is no longer active.
            println!("MOCK-SET-BLOCKED");
        }
        if self.expect_picker_screen {
            // ui::pick keeps the picker's screen: raw mode stays enabled here, and the
            // PTY tests assert no alternate-screen leave escape reaches the terminal.
            assert!(crossterm::terminal::is_raw_mode_enabled()?);
        }
        if let Some(path) = blocked_until {
            block_until_released(path, interrupted)?;
        }
        if let Some(requested) = self.cancel {
            return Err(Cancelled { requested }.into());
        }
        if self.expect_picker_screen {
            println!("MOCK-SET-{}", self.sets.get());
        }
        if self.fail || self.fail_once.replace(false) {
            bail!("Fixture rejection: HTTP=old, HTTPS=old");
        }
        self.current.borrow_mut().http = Some(browser.id.clone());
        if !self.partial {
            self.current.borrow_mut().https = Some(browser.id.clone());
        }
        Ok(())
    }
}

/// Stands in for a native setter that blocks on an OS consent dialog: bounded, and
/// stopped by the same interruption hook the real backends poll. Reaching this wait
/// means the request was already issued, so the cancellation reports `requested`.
fn block_until_released(path: &Path, interrupted: &dyn Fn() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        if interrupted() {
            // Printed from inside the setter, so the cancellation test can assert that
            // the interruption hook stopped it instead of the fixture timing out.
            println!("MOCK-SET-CANCELLED");
            return Err(Cancelled { requested: true }.into());
        }
        if Instant::now() >= deadline {
            bail!("Fixture release file was never created: {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn no_picker(_: Picker, _: &mut dyn FnMut(&mut Picker, &Browser)) -> Result<()> {
    panic!("Explicit subcommands must not invoke the picker")
}

fn rendered(picker: &Picker) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal
        .draw(|frame| {
            defbrow::ui::render(
                frame,
                picker,
                &mut ratatui::widgets::ListState::default(),
                true,
            );
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn choose(picker: &mut Picker) -> Browser {
    let PickerAction::Select(browser) =
        picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true)
    else {
        panic!("Expected selection")
    };
    browser
}

#[test]
fn parser_and_terminal_contract() {
    assert!(Cli::try_parse_from(["defbrow"]).unwrap().command.is_none());
    assert!(
        matches!(Cli::try_parse_from(["defbrow", "set", "Brave Beta"]).unwrap().command, Some(Command::Set { browser }) if browser == "Brave Beta")
    );
    assert!(Cli::try_parse_from(["defbrow", "set"]).is_err());
    assert!(Cli::try_parse_from(["defbrow", "unknown"]).is_err());
    let cli = Cli::try_parse_from(["defbrow"]).unwrap();
    assert!(cli.check_terminal(true, true).is_ok());
    for (stdin, stdout) in [(false, false), (false, true), (true, false)] {
        assert!(cli.check_terminal(stdin, stdout).is_err());
    }
    assert!(Cli::try_parse_from(["defbrow", "list"])
        .unwrap()
        .check_terminal(false, false)
        .is_ok());
}

#[test]
fn selection_requires_exact_id_or_unambiguous_case_insensitive_name() {
    let browsers = vec![
        browser("alpha.desktop", "Shared"),
        browser("beta.desktop", "Shared"),
        browser("unique.desktop", "Ünique"),
    ];
    assert_eq!(
        resolve_browser(&browsers, "alpha.desktop").unwrap().id,
        "alpha.desktop"
    );
    assert_eq!(
        resolve_browser(&browsers, "ÜNIQUE").unwrap().id,
        "unique.desktop"
    );
    assert!(resolve_browser(&browsers, "Shared")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(resolve_browser(&browsers, "alpha").is_err());
    assert!(resolve_browser(&[], "anything").is_err());
    let collision = vec![browser("Shared", "First"), browser("other", "Shared")];
    assert_eq!(resolve_browser(&collision, "Shared").unwrap().id, "Shared");
}

#[test]
fn list_and_current_show_truthful_per_scheme_state_without_setting() {
    let backend = FakeBackend {
        browsers: vec![browser("a", "Alpha"), browser("b", "Beta")],
        current: RefCell::new(CurrentDefaults {
            http: Some("a".into()),
            https: Some("b".into()),
        }),
        ..Default::default()
    };
    let mut output = Vec::new();
    execute(&backend, Some(&Command::List), &mut output, no_picker).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Alpha\ta\t[HTTP]"));
    assert!(output.contains("Beta\tb\t[HTTPS]"));
    let mut output = Vec::new();
    execute(&backend, Some(&Command::Current), &mut output, no_picker).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "HTTP: a\nHTTPS: b\n");
    assert_eq!(backend.sets.get(), 0);
}

#[test]
fn empty_list_and_current_are_explicit() {
    let backend = FakeBackend::default();
    let mut output = Vec::new();
    execute(&backend, Some(&Command::List), &mut output, no_picker).unwrap();
    assert!(String::from_utf8(output).unwrap().contains("No registered"));
    let mut output = Vec::new();
    execute(&backend, Some(&Command::Current), &mut output, no_picker).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "HTTP: (none)\nHTTPS: (none)\n"
    );
}

#[test]
fn escape_and_control_c_cancel_without_setters() {
    for key in [
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let backend = FakeBackend {
            browsers: vec![browser("a", "Alpha")],
            ..Default::default()
        };
        let mut output = Vec::new();
        execute(&backend, None, &mut output, |mut picker, _| {
            assert_eq!(picker.handle_key(key, true), PickerAction::Cancel);
            Ok(())
        })
        .unwrap();
        assert_eq!(backend.sets.get(), 0);
        assert!(output.is_empty());
    }
}

#[test]
fn selected_browser_sets_within_session_then_refreshes_and_cancels() {
    let backend = FakeBackend {
        browsers: vec![browser("a", "Alpha")],
        ..Default::default()
    };
    let mut output = Vec::new();
    execute(&backend, None, &mut output, |mut picker, switch| {
        assert_eq!(backend.sets.get(), 0);
        let PickerAction::Select(browser) =
            picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true)
        else {
            panic!("Expected selection")
        };
        switch(&mut picker, &browser);
        assert_eq!(backend.sets.get(), 1);
        assert_eq!(picker.status(), "Default browser: Alpha");
        assert!(rendered(&picker).contains("Alpha [current]"));
        assert_eq!(
            picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true),
            PickerAction::Cancel
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(backend.sets.get(), 1);
    assert!(output.is_empty());
}

#[test]
fn interactive_rejection_keeps_picker_usable_for_retry_or_cancel() {
    for retry in [false, true] {
        let backend = FakeBackend {
            browsers: vec![browser("a", "Alpha"), browser("b", "Beta")],
            current: RefCell::new(CurrentDefaults {
                http: Some("b".into()),
                https: Some("b".into()),
            }),
            fail_once: Cell::new(true),
            ..Default::default()
        };
        execute(&backend, None, &mut Vec::new(), |mut picker, switch| {
            let browser = choose(&mut picker);
            switch(&mut picker, &browser);
            assert!(picker.status().contains("Could not set Alpha"));
            let screen = rendered(&picker);
            assert!(screen.contains("Beta [current]"));
            assert!(!screen.contains("Alpha [current]"));
            if retry {
                let browser = choose(&mut picker);
                switch(&mut picker, &browser);
                assert_eq!(picker.status(), "Default browser: Alpha");
                assert!(rendered(&picker).contains("Alpha [current]"));
            }
            assert_eq!(
                picker.handle_key(
                    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                    true
                ),
                PickerAction::Cancel
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(backend.sets.get(), if retry { 2 } else { 1 });
    }
}

#[test]
fn interactive_readback_refresh_and_mismatch_are_truthful_and_recoverable() {
    for (readback_fails, refresh_fails, partial) in [
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, false, true),
    ] {
        let backend = FakeBackend {
            browsers: vec![browser("a", "Alpha"), browser("old", "Old")],
            current: RefCell::new(CurrentDefaults {
                http: Some("old".into()),
                https: Some("old".into()),
            }),
            readback_fails,
            refresh_fails,
            partial,
            ..Default::default()
        };
        let mut output = Vec::new();
        execute(&backend, None, &mut output, |mut picker, switch| {
            let browser = choose(&mut picker);
            switch(&mut picker, &browser);
            assert!(picker.status().chars().all(|c| !c.is_control()));
            let screen = rendered(&picker);
            if readback_fails {
                assert!(picker.status().contains("current unknown"));
                assert!(!picker.status().starts_with("Default browser:"));
                assert!(!screen.contains("[current]"));
            } else {
                assert!(screen.contains("Alpha [current]"));
            }
            if refresh_fails {
                assert!(picker.status().contains("previous entries shown"));
                assert_eq!(picker.visible_browsers().count(), 2);
            }
            if partial {
                assert!(picker.status().contains("not verified"));
                assert!(!picker.status().starts_with("Default browser:"));
                assert!(screen.contains("Old [current]"));
            }
            picker.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), true);
            assert_eq!(picker.selected_browser().unwrap().id, "old");
            assert_eq!(
                picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true),
                PickerAction::Cancel
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(backend.sets.get(), 1);
        assert!(output.is_empty());
    }
}

#[test]
fn interactive_refresh_preserves_filter_and_selected_identity_with_new_entries() {
    let backend = FakeBackend {
        browsers: vec![browser("a", "Alpha"), browser("b", "Beta")],
        refreshed_browsers: Some(vec![
            browser("new", "Aardvark"),
            browser("b", "Beta"),
            browser("a", "Alpha"),
        ]),
        ..Default::default()
    };
    execute(&backend, None, &mut Vec::new(), |mut picker, switch| {
        picker.set_query("a".into());
        picker.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), true);
        let browser = choose(&mut picker);
        assert_eq!(browser.id, "b");
        switch(&mut picker, &browser);
        assert_eq!(picker.query(), "a");
        assert_eq!(picker.visible_browsers().count(), 3);
        assert_eq!(picker.selected_browser().unwrap().id, "b");
        assert!(rendered(&picker).contains("Beta [current]"));
        assert_eq!(
            picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true),
            PickerAction::Cancel
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(backend.sets.get(), 1);
}

#[test]
fn cancelled_attempt_reports_the_readback_without_claiming_a_wait_that_did_not_happen() {
    for (requested, expected) in [
        (true, "Stopped waiting for approval. HTTP: b; HTTPS: b"),
        (
            false,
            "Stopped before requesting a change. HTTP: b; HTTPS: b",
        ),
    ] {
        let backend = FakeBackend {
            browsers: vec![browser("a", "Alpha"), browser("b", "Beta")],
            current: RefCell::new(CurrentDefaults {
                http: Some("b".into()),
                https: Some("b".into()),
            }),
            cancel: Some(requested),
            ..Default::default()
        };
        execute(&backend, None, &mut Vec::new(), |mut picker, switch| {
            let browser = choose(&mut picker);
            switch(&mut picker, &browser);
            assert_eq!(picker.status(), expected);
            // A stopped attempt claims neither success nor a verified change.
            assert!(!picker.status().starts_with("Default browser:"));
            assert!(rendered(&picker).contains("Beta [current]"));
            assert_eq!(
                picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true),
                PickerAction::Cancel
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(backend.sets.get(), 1);
    }
}

#[test]
fn unknown_ambiguous_and_picker_errors_do_not_set() {
    let backend = FakeBackend {
        browsers: vec![browser("a", "Shared"), browser("b", "Shared")],
        ..Default::default()
    };
    for name in ["missing", "Shared"] {
        assert!(execute(
            &backend,
            Some(&Command::Set {
                browser: name.into()
            }),
            &mut Vec::new(),
            no_picker
        )
        .is_err());
    }
    assert!(execute(&backend, None, &mut Vec::new(), |_, _| bail!(
        "Fixture terminal error"
    ))
    .is_err());
    assert_eq!(backend.sets.get(), 0);
}

#[test]
fn one_shot_set_uses_human_confirmation_and_returns_after_one_change() {
    let backend = FakeBackend {
        browsers: vec![browser("a", "Alpha")],
        ..Default::default()
    };
    let mut output = Vec::new();
    execute(
        &backend,
        Some(&Command::Set {
            browser: "a".into(),
        }),
        &mut output,
        no_picker,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output
        .starts_with("Setting default browser to Alpha. OS confirmation may be required...\n"));
    assert!(output.ends_with("Default browser: Alpha\n"));
    assert!(!output.contains("HTTP"));
    assert_eq!(backend.sets.get(), 1);
}

#[test]
fn setter_rejection_or_partial_change_never_prints_success() {
    for (fail, partial) in [(true, false), (false, true)] {
        let backend = FakeBackend {
            browsers: vec![browser("a", "Alpha")],
            current: RefCell::new(CurrentDefaults {
                http: Some("old".into()),
                https: Some("old".into()),
            }),
            fail,
            partial,
            ..Default::default()
        };
        let mut output = Vec::new();
        let error = execute(
            &backend,
            Some(&Command::Set {
                browser: "a".into(),
            }),
            &mut output,
            no_picker,
        )
        .unwrap_err();
        assert!(!String::from_utf8(output)
            .unwrap()
            .contains("Default browser:"));
        if partial {
            assert!(error.to_string().contains("HTTP=a, HTTPS=old"));
        } else {
            assert!(error.to_string().contains("Fixture rejection"));
        }
    }
}

#[cfg(unix)]
#[test]
fn native_picker_stays_in_same_process_after_success_or_failure_and_restores_terminal() {
    for mode in ["success", "reject-once", "reject-cancel"] {
        let mut session = pty::Session::start(mode);
        session.wait_for_all(&["Search", "Beta [current]"]);
        session.send(b"\r");
        // The retained picker screen makes ratatui redraw only changed cells, so the
        // wait and the result status are matched instead of unchanged list markers.
        session.wait_for_all(if mode == "success" {
            &[
                "MOCK-SET-1",
                defbrow::ui::WAITING_STATUS,
                "Default browser: Alpha",
            ]
        } else {
            &[
                "MOCK-SET-1",
                defbrow::ui::WAITING_STATUS,
                "Could not set Alpha",
            ]
        });
        if mode != "reject-cancel" {
            // A second Enter must reach a second fake setter within this same session.
            session.send(b"\r");
            session.wait_for_all(&["MOCK-SET-2", "Default browser: Alpha"]);
        }
        assert!(
            !session.raw_output().contains("\u{1b}[?1049l"),
            "the setter must not leave the alternate screen"
        );
        session.send(if mode == "reject-once" {
            b"\x03"
        } else {
            b"\x1b"
        });
        session.finish();
    }
}

#[cfg(unix)]
#[test]
fn native_picker_keeps_its_screen_while_the_setter_waits_for_approval() {
    let mut session = pty::Session::start_blocked("block-success");
    session.wait_for_all(&["Search", "Alpha", "Beta [current]"]);
    session.send(b"\r");
    session.wait_for_all(&[defbrow::ui::WAITING_STATUS, "MOCK-SET-BLOCKED"]);
    // The wait is on screen while the setter blocks. ratatui redraws only changed
    // cells, so the picker screen drawn earlier is matched on the complete raw
    // transcript instead of the latest output window.
    let raw = session.raw_output();
    assert!(
        raw.contains("\u{1b}[?1049h"),
        "the picker must enter the alternate screen"
    );
    assert!(
        !raw.contains("\u{1b}[?1049l"),
        "the picker must not leave the alternate screen while the setter waits"
    );
    session.release();
    session.wait_for_all(&["MOCK-SET-1", "Default browser: Alpha"]);
    session.send(b"\x1b");
    session.finish();
    assert!(
        session.raw_output().contains("\u{1b}[?1049l"),
        "the terminal must be restored on exit"
    );
}

#[cfg(unix)]
#[test]
fn cancelled_wait_keeps_the_picker_open_and_reports_the_real_state() {
    let mut session = pty::Session::start_blocked("block-cancel");
    session.wait_for_all(&["Search", "Beta [current]"]);
    session.send(b"\r");
    session.wait_for_all(&[defbrow::ui::WAITING_STATUS, "MOCK-SET-BLOCKED"]);
    // Ctrl-C stops the wait promptly; the fake setter would otherwise block for 30s.
    session.send(b"\x03");
    // The setter's own marker is the primary cancellation signal: waiting on the status
    // text alone would only prove the wait ended, not that the hook stopped it.
    session.wait_for_all(&[
        "MOCK-SET-CANCELLED",
        "Stopped waiting for approval.",
        "HTTP: b; HTTPS: b",
    ]);
    assert!(
        !session.raw_output().contains("\u{1b}[?1049l"),
        "the picker must not leave the alternate screen to stop waiting"
    );
    // The picker is still live: typing a non-matching query redraws the list.
    session.send(b"z");
    session.wait_for("No matching browsers. Ctrl-U clears.");
    session.send(b"\x1b");
    session.finish();
    assert!(
        session.raw_output().contains("\u{1b}[?1049l"),
        "the terminal must be restored on exit"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "subprocess fixture invoked only through a fake-backend PTY session"]
fn native_picker_fixture() {
    let mode = std::env::var("DEFBROW_TEST_PICKER").expect("PTY fixture requires its test parent");
    let backend = FakeBackend {
        browsers: vec![browser("a", "Alpha"), browser("b", "Beta")],
        current: RefCell::new(CurrentDefaults {
            http: Some("b".into()),
            https: Some("b".into()),
        }),
        fail_once: Cell::new(mode.starts_with("reject")),
        expect_picker_screen: true,
        block_until_released: std::env::var_os("DEFBROW_TEST_RELEASE_FILE").map(PathBuf::from),
        ..Default::default()
    };
    execute(&backend, None, &mut Vec::new(), defbrow::ui::pick).unwrap();
    assert_eq!(
        backend.sets.get(),
        match mode.as_str() {
            "success" | "reject-once" => 2,
            "reject-cancel" | "block-success" | "block-cancel" => 1,
            other => panic!("unknown fixture mode: {other}"),
        }
    );
    assert!(!crossterm::terminal::is_raw_mode_enabled().unwrap());
    println!("MOCK-SESSION-CANCELLED");
}

#[test]
fn binary_help_version_and_non_tty_are_safe() {
    for argument in ["--help", "--version"] {
        let result = ProcessCommand::new(env!("CARGO_BIN_EXE_defbrow"))
            .arg(argument)
            .output()
            .unwrap();
        assert!(result.status.success());
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(output.contains(if argument == "--help" {
            "Usage:"
        } else {
            "0.1.0"
        }));
    }
    let result = ProcessCommand::new(env!("CARGO_BIN_EXE_defbrow"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(error.contains("needs a terminal"));
    assert!(error.contains("defbrow list"));
    assert!(!error.contains("not implemented"));
}
