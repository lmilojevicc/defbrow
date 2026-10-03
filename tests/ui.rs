use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use defbrow::{
    backend::{Browser, CurrentDefaults},
    ui::{self, Picker, PickerAction},
};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    widgets::ListState,
    Terminal,
};

fn browser(id: &str, name: &str, detail: &str) -> Browser {
    Browser {
        id: id.into(),
        name: name.into(),
        detail: detail.into(),
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn fuzzy_subsequence_is_case_insensitive_unicode_and_ranked() {
    assert!(ui::fuzzy_score("brv", "Brave").is_some());
    assert!(ui::fuzzy_score("lbrwf", "LibreWolf").is_some());
    assert!(ui::fuzzy_score("lwf", "LibreWolf").is_some());
    assert!(ui::fuzzy_score("ÜB", "Über Browser").is_some());
    assert!(ui::fuzzy_score("ff", "Firefox").is_some());
    assert!(ui::fuzzy_score("bbb", "Brave").is_none());
    assert!(ui::fuzzy_score("vb", "Brave").is_none());
    assert!(ui::fuzzy_score("brv", "Brave") < ui::fuzzy_score("brv", "Big Rare Village"));
    assert!(ui::fuzzy_score("brave", "Brave") < ui::fuzzy_score("brave", "Brave Beta"));
    assert_eq!(ui::fuzzy_score("", ""), Some((0, 0)));
    assert_eq!(ui::fuzzy_score("x", ""), None);
}

#[test]
fn picker_searches_all_fields_and_ranks_deterministically() {
    let mut picker = Picker::new(
        vec![
            browser("id.one", "Big Rare Village", ""),
            browser("id.two", "Brave", ""),
            browser("org.librewolf", "Other", ""),
            browser("id.four", "Another", "/opt/Über Browser"),
            browser("id.five", "Brave", ""),
        ],
        CurrentDefaults::default(),
    );
    picker.set_query("BRV".into());
    assert_eq!(
        picker
            .visible_browsers()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>(),
        ["id.two", "id.five", "id.one"]
    );
    picker.set_query("lbrwf".into());
    assert_eq!(picker.selected_browser().unwrap().id, "org.librewolf");
    picker.set_query("Üb".into());
    assert_eq!(picker.selected_browser().unwrap().id, "id.four");
    picker.set_query(String::new());
    assert_eq!(
        picker
            .visible_browsers()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>(),
        ["id.one", "id.two", "org.librewolf", "id.four", "id.five"]
    );
}

#[test]
fn navigation_wraps_and_filtering_resets_selection() {
    let first = browser("a", "Alpha", "");
    let second = browser("b", "Beta", "");
    let mut picker = Picker::new(
        vec![first.clone(), second.clone()],
        CurrentDefaults::default(),
    );
    picker.handle_key(key(KeyCode::Up), true);
    assert_eq!(picker.selected_browser(), Some(&second));
    picker.handle_key(key(KeyCode::Down), true);
    assert_eq!(picker.selected_browser(), Some(&first));
    picker.handle_key(key(KeyCode::Down), true);
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter), true),
        PickerAction::Select(second)
    );
    picker.set_query("Alpha".into());
    assert_eq!(picker.selected_browser(), Some(&first));
}

#[test]
fn unicode_backspace_clear_cancel_and_key_releases() {
    let mut picker = Picker::new(vec![browser("a", "Über", "")], CurrentDefaults::default());
    picker.handle_key(key(KeyCode::Char('Ü')), true);
    picker.handle_key(key(KeyCode::Char('b')), true);
    assert_eq!(picker.query(), "Üb");
    picker.handle_key(key(KeyCode::Backspace), true);
    assert_eq!(picker.query(), "Ü");
    picker.handle_key(
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        true,
    );
    assert_eq!(picker.query(), "");
    let mut release = key(KeyCode::Char('x'));
    release.kind = KeyEventKind::Release;
    picker.handle_key(release, true);
    assert_eq!(picker.query(), "");
    assert_eq!(
        picker.handle_key(key(KeyCode::Esc), true),
        PickerAction::Cancel
    );
    assert_eq!(
        picker.handle_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            true
        ),
        PickerAction::Cancel
    );
}

#[test]
fn empty_no_matches_and_hidden_selection_cannot_confirm() {
    let mut picker = Picker::new(Vec::new(), CurrentDefaults::default());
    for code in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Backspace,
    ] {
        assert_eq!(picker.handle_key(key(code), true), PickerAction::Continue);
    }
    let mut picker = Picker::new(vec![browser("a", "Alpha", "")], CurrentDefaults::default());
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter), false),
        PickerAction::Continue
    );
    picker.set_query("zz".into());
    assert_eq!(picker.selected_browser(), None);
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter), true),
        PickerAction::Continue
    );
}

#[test]
fn current_markers_are_per_scheme() {
    let browser = browser("a", "Alpha", "");
    assert_eq!(
        ui::current_marker(&browser, &CurrentDefaults::default()),
        ""
    );
    assert_eq!(
        ui::current_marker(
            &browser,
            &CurrentDefaults {
                http: Some("a".into()),
                https: Some("b".into())
            }
        ),
        "[HTTP]"
    );
    assert_eq!(
        ui::current_marker(
            &browser,
            &CurrentDefaults {
                http: Some("b".into()),
                https: Some("a".into())
            }
        ),
        "[HTTPS]"
    );
    assert_eq!(
        ui::current_marker(
            &browser,
            &CurrentDefaults {
                http: Some("a".into()),
                https: Some("a".into())
            }
        ),
        "[HTTP HTTPS]"
    );
}

fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn render_inherits_defaults_uses_ansi_accent_and_reversed_selection() {
    let picker = Picker::new(
        vec![browser("a", "Alpha", "detail")],
        CurrentDefaults::default(),
    );
    for no_color in [false, true] {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut state = ListState::default();
        terminal
            .draw(|frame| assert!(ui::render(frame, &picker, &mut state, no_color)))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert!(text(buffer).contains("> Alpha"));
        assert_eq!(
            buffer[(0, 0)].fg,
            if no_color { Color::Reset } else { Color::Cyan }
        );
        assert!(buffer.content.iter().all(|cell| cell.bg == Color::Reset));
        assert!(buffer
            .content
            .iter()
            .all(|cell| matches!(cell.fg, Color::Reset | Color::Cyan)));
        if no_color {
            assert!(buffer.content.iter().all(|cell| cell.fg == Color::Reset));
        }
        assert!(buffer
            .content
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED)));
        assert_eq!(ui::selection_style().fg, None);
        assert_eq!(ui::selection_style().bg, None);
    }
}

#[test]
fn render_handles_empty_no_match_and_resize() {
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut state = ListState::default();
    let empty = Picker::new(Vec::new(), CurrentDefaults::default());
    terminal
        .draw(|frame| {
            ui::render(frame, &empty, &mut state, true);
        })
        .unwrap();
    assert!(text(terminal.backend().buffer()).contains("No registered HTTP+HTTPS handlers."));
    let mut picker = Picker::new(vec![browser("a", "Alpha", "")], CurrentDefaults::default());
    picker.set_query("zzz".into());
    terminal
        .draw(|frame| {
            ui::render(frame, &picker, &mut state, true);
        })
        .unwrap();
    assert!(text(terminal.backend().buffer()).contains("No matching browsers."));
    terminal.backend_mut().resize(20, 5);
    terminal
        .draw(|frame| assert!(!ui::render(frame, &picker, &mut state, true)))
        .unwrap();
    assert!(text(terminal.backend().buffer()).contains("Resize"));
    terminal.backend_mut().resize(80, 20);
    terminal
        .draw(|frame| assert!(ui::render(frame, &picker, &mut state, true)))
        .unwrap();
    assert!(text(terminal.backend().buffer()).contains("No matching browsers."));
}

#[test]
fn render_does_not_emit_control_sequences_from_metadata() {
    let picker = Picker::new(
        vec![browser("a", "Alpha\u{1b}[2J", "bad\nline")],
        CurrentDefaults::default(),
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal
        .draw(|frame| {
            ui::render(frame, &picker, &mut ListState::default(), true);
        })
        .unwrap();
    assert!(!text(terminal.backend().buffer()).contains('\u{1b}'));
}
