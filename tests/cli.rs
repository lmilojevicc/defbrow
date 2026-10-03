use std::{
    cell::{Cell, RefCell},
    process::Command as ProcessCommand,
};

use anyhow::{bail, Result};
use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use defbrow::{
    backend::{Backend, Browser, CurrentDefaults},
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
}

impl Backend for FakeBackend {
    fn browsers(&self) -> Result<Vec<Browser>> {
        Ok(self.browsers.clone())
    }
    fn current(&self) -> Result<CurrentDefaults> {
        Ok(self.current.borrow().clone())
    }
    fn set_default(&self, browser: &Browser) -> Result<()> {
        self.sets.set(self.sets.get() + 1);
        if self.fail {
            bail!("Fixture rejection: HTTP=old, HTTPS=old");
        }
        self.current.borrow_mut().http = Some(browser.id.clone());
        if !self.partial {
            self.current.borrow_mut().https = Some(browser.id.clone());
        }
        Ok(())
    }
}

fn no_picker(_: Vec<Browser>, _: CurrentDefaults) -> Result<Option<Browser>> {
    panic!("Explicit subcommands must not invoke the picker")
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
        execute(&backend, None, &mut output, |browsers, current| {
            let mut picker = Picker::new(browsers, current);
            assert_eq!(picker.handle_key(key, true), PickerAction::Cancel);
            Ok(None)
        })
        .unwrap();
        assert_eq!(backend.sets.get(), 0);
        assert!(output.is_empty());
    }
}

#[test]
fn selected_browser_sets_only_after_picker_returns() {
    let backend = FakeBackend {
        browsers: vec![browser("a", "Alpha")],
        ..Default::default()
    };
    let mut output = Vec::new();
    let returned = Cell::new(false);
    execute(&backend, None, &mut output, |browsers, current| {
        let mut picker = Picker::new(browsers, current);
        assert_eq!(backend.sets.get(), 0);
        let PickerAction::Select(browser) =
            picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true)
        else {
            panic!("Expected selection")
        };
        returned.set(true);
        Ok(Some(browser))
    })
    .unwrap();
    assert!(returned.get());
    assert_eq!(backend.sets.get(), 1);
    assert!(String::from_utf8(output)
        .unwrap()
        .contains("Default browser: Alpha"));
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
