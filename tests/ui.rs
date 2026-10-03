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

fn render_buffer(picker: &Picker, no_color: bool) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal
        .draw(|frame| {
            assert!(ui::render(
                frame,
                picker,
                &mut ListState::default(),
                no_color
            ));
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn colored_text(buffer: &Buffer, color: Color) -> String {
    buffer
        .content
        .iter()
        .filter(|cell| cell.fg == color)
        .map(|cell| cell.symbol())
        .collect()
}

fn assert_no_protocol_labels(buffer: &Buffer) {
    let rendered = text(buffer);
    assert!(!rendered.to_ascii_uppercase().contains("HTTP"));
    assert!(!rendered.contains("fuzzy"));
    assert!(!rendered.contains("backend"));
}

#[test]
fn render_colors_title_borders_search_selection_and_current_with_ansi_palette() {
    let picker = Picker::new(
        vec![
            browser("raw.alpha.id", "Alpha", "/path/to/Alpha.app"),
            browser("raw.beta.id", "Beta", "/path/to/Beta.app"),
        ],
        CurrentDefaults {
            http: Some("raw.beta.id".into()),
            https: Some("raw.beta.id".into()),
        },
    );
    let buffer = render_buffer(&picker, false);
    assert!(colored_text(&buffer, Color::Cyan).contains("defbrow — choose a browser"));
    assert!(buffer
        .content
        .iter()
        .any(|cell| cell.symbol() == "│" && cell.fg == Color::Cyan));
    assert!(colored_text(&buffer, Color::Yellow).contains("Search"));
    assert!(!text(&buffer).contains("(fuzzy)"));
    assert!(colored_text(&buffer, Color::Cyan).contains("> Alpha"));
    assert!(colored_text(&buffer, Color::Green).contains("[current]"));
    assert!(buffer.content.iter().all(|cell| cell.bg == Color::Reset));
    assert!(buffer.content.iter().all(|cell| matches!(
        cell.fg,
        Color::Reset | Color::Cyan | Color::Yellow | Color::Green
    )));
    assert!(buffer
        .content
        .iter()
        .filter(|cell| cell.modifier.contains(Modifier::REVERSED))
        .any(|cell| cell.symbol() == "A" && cell.fg == Color::Cyan));
    assert_no_protocol_labels(&buffer);
    assert!(!text(&buffer).contains("raw."));
    assert!(!text(&buffer).contains("/path/to/"));
}

#[test]
fn render_no_color_inherits_palette_and_keeps_selection_and_current_distinct() {
    let picker = Picker::new(
        vec![browser("a", "Alpha", ""), browser("b", "Beta", "")],
        CurrentDefaults {
            http: Some("b".into()),
            https: Some("b".into()),
        },
    );
    let buffer = render_buffer(&picker, true);
    assert!(buffer
        .content
        .iter()
        .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset));
    assert!(text(&buffer).contains("> Alpha"));
    assert!(text(&buffer).contains("Beta [current]"));
    assert!(buffer
        .content
        .iter()
        .any(|cell| cell.modifier.contains(Modifier::REVERSED)));
    assert!(buffer
        .content
        .iter()
        .any(|cell| cell.symbol() == "[" && cell.modifier.contains(Modifier::BOLD)));
    assert_eq!(ui::selection_style().fg, None);
    assert_eq!(ui::selection_style().bg, None);
    assert_no_protocol_labels(&buffer);
}

#[test]
fn render_uses_one_current_marker_for_same_split_missing_and_unknown_defaults() {
    for (http, https, marked) in [
        (Some("raw.alpha.id"), Some("raw.alpha.id"), vec!["Alpha"]),
        (
            Some("raw.alpha.id"),
            Some("raw.beta.id"),
            vec!["Alpha", "Beta"],
        ),
        (Some("raw.alpha.id"), None, vec!["Alpha"]),
        (None, Some("raw.beta.id"), vec!["Beta"]),
        (None, None, vec![]),
        (Some("raw.unknown.id"), Some("raw.unknown.id"), vec![]),
    ] {
        let picker = Picker::new(
            vec![
                browser("raw.alpha.id", "Alpha", ""),
                browser("raw.beta.id", "Beta", ""),
            ],
            CurrentDefaults {
                http: http.map(str::to_owned),
                https: https.map(str::to_owned),
            },
        );
        for no_color in [false, true] {
            let buffer = render_buffer(&picker, no_color);
            let rendered = text(&buffer);
            assert_eq!(rendered.matches("[current]").count(), marked.len());
            for name in &marked {
                assert!(rendered.contains(&format!("{name} [current]")));
            }
            assert_no_protocol_labels(&buffer);
            assert!(!rendered.contains("raw."));
        }
    }
}

#[test]
fn render_only_shows_details_to_disambiguate_duplicate_names() {
    let mut picker = Picker::new(
        vec![
            browser("a", "Alpha", "/Applications/Alpha.app"),
            browser("b", "Alpha", "/Users/me/Applications/Alpha.app"),
            browser("beta-one", "Beta", "shared-detail"),
            browser("beta-two", "Beta", "shared-detail"),
            browser("g", "Gamma", "hidden-detail"),
        ],
        CurrentDefaults::default(),
    );
    let buffer = render_buffer(&picker, false);
    let rendered = text(&buffer);
    assert!(rendered.contains("Alpha — /Applications/Alpha.app"));
    assert!(rendered.contains("Alpha — /Users/me/Applications/Alpha.app"));
    assert!(rendered.contains("Beta — beta-one"));
    assert!(rendered.contains("Beta — beta-two"));
    assert!(!rendered.contains("hidden-detail"));
    assert!(!rendered.contains("shared-detail"));
    assert_no_protocol_labels(&buffer);
    picker.set_query("/Users/me".into());
    assert!(
        text(&render_buffer(&picker, false)).contains("Alpha — /Users/me/Applications/Alpha.app")
    );
}

#[test]
fn render_handles_empty_no_match_and_resize() {
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut state = ListState::default();
    let empty = Picker::new(
        Vec::new(),
        CurrentDefaults {
            http: Some("raw.unknown.id".into()),
            https: Some("raw.unknown.id".into()),
        },
    );
    for no_color in [false, true] {
        terminal
            .draw(|frame| {
                ui::render(frame, &empty, &mut state, no_color);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert!(text(buffer).contains("No browsers found."));
        assert_no_protocol_labels(buffer);
        assert!(!text(buffer).contains("raw.unknown.id"));
    }
    let mut picker = Picker::new(vec![browser("a", "Alpha", "")], CurrentDefaults::default());
    picker.set_query("zzz".into());
    terminal
        .draw(|frame| {
            ui::render(frame, &picker, &mut state, true);
        })
        .unwrap();
    assert!(text(terminal.backend().buffer()).contains("No matching browsers."));
    assert_no_protocol_labels(terminal.backend().buffer());
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
        vec![
            browser("a", "Alpha\u{1b}[2J", "bad\nline"),
            browser("b", "Alpha\u{1b}[2J", "other\u{1b}[2J"),
        ],
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
