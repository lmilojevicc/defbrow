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
    assert!(ui::is_cancel_key(&key(KeyCode::Esc)));
    assert!(ui::is_cancel_key(&KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL
    )));
    assert!(!ui::is_cancel_key(&key(KeyCode::Char('c'))));
    let mut released_escape = key(KeyCode::Esc);
    released_escape.kind = KeyEventKind::Release;
    assert!(!ui::is_cancel_key(&released_escape));
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
    render_buffer_at_size(picker, no_color, 100, 20)
}

fn render_buffer_at_size(picker: &Picker, no_color: bool, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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

fn assert_text_color(buffer: &Buffer, x: u16, y: u16, expected: &str, color: Color) {
    for (offset, character) in expected.chars().enumerate() {
        let cell = &buffer[(x + offset as u16, y)];
        assert_eq!(cell.symbol(), character.to_string());
        assert_eq!(
            cell.fg,
            color,
            "wrong color at ({}, {y})",
            x + offset as u16
        );
    }
}

fn assert_rail(buffer: &Buffer, y: u16, name: &str, no_color: bool) {
    assert_text_color(
        buffer,
        2,
        y,
        "▌",
        if no_color { Color::Reset } else { Color::Red },
    );
    assert_text_color(buffer, 3, y, &format!(" {name}"), Color::Reset);
}

fn assert_no_protocol_labels(buffer: &Buffer) {
    let rendered = text(buffer);
    assert!(!rendered.to_ascii_uppercase().contains("HTTP"));
    assert!(!rendered.contains("fuzzy"));
    assert!(!rendered.contains("backend"));
    assert!(!rendered.contains("defbrow"));
    assert!(!rendered.contains("choose a browser"));
}

fn assert_outer_inset(buffer: &Buffer) {
    let area = buffer.area;
    for y in 0..area.height {
        for x in 0..area.width {
            if x == 0 || x == area.width - 1 || y == 0 || y == area.height - 1 {
                let cell = &buffer[(x, y)];
                assert_eq!(cell.symbol(), " ", "outer inset at ({x}, {y})");
                assert_eq!(cell.fg, Color::Reset);
                assert_eq!(cell.bg, Color::Reset);
                assert!(cell.modifier.is_empty());
            }
        }
    }
}

fn assert_selection_background(buffer: &Buffer, selected_y: Option<u16>, no_color: bool) {
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let expected =
                if !no_color && Some(y) == selected_y && (2..buffer.area.width - 2).contains(&x) {
                    Color::Indexed(237)
                } else {
                    Color::Reset
                };
            assert_eq!(buffer[(x, y)].bg, expected, "background at ({x}, {y})");
            assert!(!buffer[(x, y)].modifier.contains(Modifier::REVERSED));
        }
    }
}

#[test]
fn render_insets_widgets_and_uses_normal_text_with_tag_only_yellow() {
    let mut picker = Picker::new(
        vec![
            browser("raw.alpha.id", "Alpha", "/path/to/Alpha.app"),
            browser("raw.beta.id", "Beta", "/path/to/Beta.app"),
        ],
        CurrentDefaults {
            http: Some("raw.beta.id".into()),
            https: Some("raw.beta.id".into()),
        },
    );
    picker.set_query("a".into());
    let buffer = render_buffer(&picker, false);
    assert_outer_inset(&buffer);
    assert_text_color(&buffer, 1, 1, "┌", Color::DarkGray);
    assert_text_color(&buffer, 2, 1, "─ ", Color::DarkGray);
    assert_text_color(&buffer, 4, 1, "Search", Color::LightMagenta);
    assert_text_color(&buffer, 10, 1, "─", Color::DarkGray);
    assert_text_color(&buffer, 2, 2, "a", Color::Reset);
    assert_text_color(&buffer, 1, 4, "┌", Color::DarkGray);
    assert_text_color(&buffer, 2, 4, "─ ", Color::DarkGray);
    assert_text_color(&buffer, 4, 4, "Browsers (2)", Color::Green);
    assert_text_color(&buffer, 16, 4, "─", Color::DarkGray);
    assert_text_color(&buffer, 2, 5, "▌", Color::Red);
    assert_text_color(&buffer, 3, 5, " Alpha", Color::Reset);
    assert_text_color(&buffer, 4, 6, "Beta ", Color::Reset);
    assert_text_color(&buffer, 9, 6, "[current]", Color::Yellow);
    assert_text_color(
        &buffer,
        1,
        18,
        "↑/↓ move · Enter set · Esc cancel · Ctrl-U clear",
        Color::DarkGray,
    );
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            assert_eq!(cell.fg == Color::Yellow, y == 6 && (9..18).contains(&x));
            assert_eq!(cell.fg == Color::Red, y == 5 && x == 2);
            assert_eq!(cell.fg == Color::Green, y == 4 && (4..16).contains(&x));
            assert_eq!(
                cell.fg == Color::LightMagenta,
                y == 1 && (4..10).contains(&x)
            );
            assert!(matches!(
                cell.fg,
                Color::Reset
                    | Color::DarkGray
                    | Color::Yellow
                    | Color::Green
                    | Color::LightMagenta
                    | Color::Red
            ));
        }
    }
    assert_selection_background(&buffer, Some(5), false);
    assert_no_protocol_labels(&buffer);
    assert!(!text(&buffer).contains("raw."));
    assert!(!text(&buffer).contains("/path/to/"));
}

#[test]
fn render_selected_current_tag_stays_yellow_without_coloring_name_or_spacing() {
    let mut picker = Picker::new(
        vec![browser("a", "Alpha", ""), browser("b", "Beta", "")],
        CurrentDefaults {
            http: Some("b".into()),
            https: Some("b".into()),
        },
    );
    picker.handle_key(key(KeyCode::Down), true);
    let buffer = render_buffer(&picker, false);
    assert_text_color(&buffer, 2, 5, "  Alpha", Color::Reset);
    assert_text_color(&buffer, 2, 6, "▌", Color::Red);
    assert_text_color(&buffer, 3, 6, " Beta ", Color::Reset);
    assert_text_color(&buffer, 9, 6, "[current]", Color::Yellow);
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            assert_eq!(
                buffer[(x, y)].fg == Color::Yellow,
                y == 6 && (9..18).contains(&x)
            );
        }
    }
    assert_selection_background(&buffer, Some(6), false);
    assert!(buffer[(9, 6)].modifier.contains(Modifier::BOLD));
}

#[test]
fn render_preserves_list_and_footer_inside_inset_at_minimum_usable_size() {
    let mut picker = Picker::new(
        vec![
            browser("a", "Alpha", ""),
            browser("b", "Beta", ""),
            browser("c", "Gamma", ""),
            browser("d", "Delta", ""),
            browser("e", "Epsilon", ""),
        ],
        CurrentDefaults::default(),
    );
    for no_color in [false, true] {
        let buffer = render_buffer_at_size(&picker, no_color, 32, 10);
        let border_color = if no_color {
            Color::Reset
        } else {
            Color::DarkGray
        };
        assert_outer_inset(&buffer);
        assert_text_color(
            &buffer,
            4,
            1,
            "Search",
            if no_color {
                Color::Reset
            } else {
                Color::LightMagenta
            },
        );
        assert_text_color(
            &buffer,
            4,
            4,
            "Browsers (5)",
            if no_color { Color::Reset } else { Color::Green },
        );
        for (y, title_end) in [(1, 10), (4, 16)] {
            assert_text_color(&buffer, 1, y, "┌─ ", border_color);
            assert_text_color(
                &buffer,
                title_end,
                y,
                &"─".repeat(usize::from(30 - title_end)),
                border_color,
            );
            assert_text_color(&buffer, 30, y, "┐", border_color);
        }
        assert_text_color(&buffer, 2, 2, " ", Color::Reset);
        assert_text_color(&buffer, 1, 3, "└", border_color);
        assert_rail(&buffer, 5, "Alpha", no_color);
        assert_text_color(&buffer, 4, 6, "Beta", Color::Reset);
        assert_text_color(&buffer, 1, 7, "└", border_color);
        assert_text_color(&buffer, 1, 8, "↑/↓ move", border_color);
        assert!(!text(&buffer).contains("Gamma"));
        assert_selection_background(&buffer, Some(5), no_color);
        assert_no_protocol_labels(&buffer);
    }
    picker.handle_key(key(KeyCode::Up), true);
    for no_color in [false, true] {
        let buffer = render_buffer_at_size(&picker, no_color, 32, 10);
        assert_rail(&buffer, 6, "Epsilon", no_color);
        assert_selection_background(&buffer, Some(6), no_color);
        assert_outer_inset(&buffer);
    }
}

#[test]
fn render_no_color_inherits_palette_and_keeps_selection_and_current_distinct() {
    let mut picker = Picker::new(
        vec![
            browser("a", "Alpha", "first-copy"),
            browser("b", "Beta", ""),
        ],
        CurrentDefaults {
            http: Some("b".into()),
            https: Some("b".into()),
        },
    );
    picker.set_query("a".into());
    for (selected_y, pointer_text) in [(5, "▌ Alpha"), (6, "▌ Beta")] {
        let buffer = render_buffer(&picker, true);
        assert_text_color(&buffer, 2, 1, "─ ", Color::Reset);
        assert_text_color(&buffer, 4, 1, "Search", Color::Reset);
        assert_text_color(&buffer, 2, 4, "─ ", Color::Reset);
        assert_text_color(&buffer, 4, 4, "Browsers (2)", Color::Reset);
        assert_text_color(&buffer, 2, 2, "a", Color::Reset);
        assert_text_color(&buffer, 2, selected_y, pointer_text, Color::Reset);
        assert_text_color(&buffer, 9, 6, "[current]", Color::Reset);
        assert!(buffer
            .content
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset));
        assert!(buffer[(9, 6)].modifier.contains(Modifier::BOLD));
        assert_selection_background(&buffer, Some(selected_y), true);
        assert_outer_inset(&buffer);
        assert_no_protocol_labels(&buffer);
        picker.handle_key(key(KeyCode::Down), true);
    }
    assert_eq!(ui::selection_style(true).fg, None);
    assert_eq!(ui::selection_style(true).bg, None);
    assert_eq!(ui::selection_style(false).fg, None);
    assert_eq!(ui::selection_style(false).bg, Some(Color::Indexed(237)));
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
    for no_color in [false, true] {
        let buffer = render_buffer(&picker, no_color);
        let detail_color = if no_color {
            Color::Reset
        } else {
            Color::DarkGray
        };
        let rendered = text(&buffer);
        assert!(rendered.contains("Alpha — /Applications/Alpha.app"));
        assert!(rendered.contains("Alpha — /Users/me/Applications/Alpha.app"));
        assert!(rendered.contains("Beta — beta-one"));
        assert!(rendered.contains("Beta — beta-two"));
        assert!(!rendered.contains("hidden-detail"));
        assert!(!rendered.contains("shared-detail"));
        assert_no_protocol_labels(&buffer);
        assert_text_color(&buffer, 4, 5, "Alpha", Color::Reset);
        assert_text_color(&buffer, 9, 5, " — /Applications/Alpha.app", Color::Reset);
        assert_text_color(&buffer, 4, 6, "Alpha", Color::Reset);
        assert_text_color(
            &buffer,
            9,
            6,
            " — /Users/me/Applications/Alpha.app",
            detail_color,
        );
        assert!(buffer[(12, 5)].modifier.contains(Modifier::DIM));
        assert!(buffer[(12, 6)].modifier.contains(Modifier::DIM));
        assert_outer_inset(&buffer);
        assert_selection_background(&buffer, Some(5), no_color);
    }
    picker.handle_key(key(KeyCode::Down), true);
    for no_color in [false, true] {
        let buffer = render_buffer(&picker, no_color);
        assert_text_color(
            &buffer,
            9,
            5,
            " — /Applications/Alpha.app",
            if no_color {
                Color::Reset
            } else {
                Color::DarkGray
            },
        );
        assert_rail(&buffer, 6, "Alpha", no_color);
        assert_text_color(
            &buffer,
            9,
            6,
            " — /Users/me/Applications/Alpha.app",
            Color::Reset,
        );
        assert_selection_background(&buffer, Some(6), no_color);
    }
    picker.set_query("/Users/me".into());
    let buffer = render_buffer(&picker, false);
    assert_rail(&buffer, 5, "Alpha", false);
    assert_text_color(
        &buffer,
        9,
        5,
        " — /Users/me/Applications/Alpha.app",
        Color::Reset,
    );
    assert_selection_background(&buffer, Some(5), false);
}

#[test]
fn render_selected_duplicate_details_contrast_with_shading_and_preserve_current_tag() {
    let mut picker = Picker::new(
        vec![
            browser("a", "Alpha", "first-copy"),
            browser("b", "Alpha", "second-copy"),
        ],
        CurrentDefaults {
            http: Some("a".into()),
            https: Some("a".into()),
        },
    );
    for selected_y in [5, 6] {
        for no_color in [false, true] {
            let buffer = render_buffer(&picker, no_color);
            assert_text_color(&buffer, 4, 5, "Alpha ", Color::Reset);
            assert_text_color(
                &buffer,
                10,
                5,
                "[current]",
                if no_color {
                    Color::Reset
                } else {
                    Color::Yellow
                },
            );
            for (y, x, detail) in [(5, 19, " — first-copy"), (6, 9, " — second-copy")] {
                assert_text_color(
                    &buffer,
                    x,
                    y,
                    detail,
                    if no_color || y == selected_y {
                        Color::Reset
                    } else {
                        Color::DarkGray
                    },
                );
            }
            assert_rail(&buffer, selected_y, "Alpha", no_color);
            assert_selection_background(&buffer, Some(selected_y), no_color);
            assert_outer_inset(&buffer);
        }
        picker.handle_key(key(KeyCode::Down), true);
    }
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
                assert!(ui::render(frame, &empty, &mut state, no_color));
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert!(text(buffer).contains("No browsers found."));
        assert_text_color(
            buffer,
            2,
            5,
            "No browsers found.",
            if no_color {
                Color::Reset
            } else {
                Color::DarkGray
            },
        );
        assert_no_protocol_labels(buffer);
        assert!(!text(buffer).contains("raw.unknown.id"));
        assert_outer_inset(buffer);
        assert_selection_background(buffer, None, no_color);
        assert!(!text(buffer).contains('>'));
    }
    let mut picker = Picker::new(vec![browser("a", "Alpha", "")], CurrentDefaults::default());
    picker.set_query("zzz".into());
    for no_color in [false, true] {
        terminal
            .draw(|frame| assert!(ui::render(frame, &picker, &mut state, no_color)))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_text_color(
            buffer,
            2,
            5,
            "No matching browsers. Ctrl-U clears.",
            if no_color {
                Color::Reset
            } else {
                Color::DarkGray
            },
        );
        assert_no_protocol_labels(buffer);
        assert_outer_inset(buffer);
        assert_selection_background(buffer, None, no_color);
        assert!(!text(buffer).contains('>'));
    }
    terminal.backend_mut().resize(20, 5);
    for no_color in [false, true] {
        terminal
            .draw(|frame| assert!(!ui::render(frame, &picker, &mut state, no_color)))
            .unwrap();
        assert_text_color(
            terminal.backend().buffer(),
            1,
            1,
            "Resize",
            if no_color {
                Color::Reset
            } else {
                Color::DarkGray
            },
        );
        assert_outer_inset(terminal.backend().buffer());
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .all(|cell| cell.bg == Color::Reset));
    }
    terminal.backend_mut().resize(80, 20);
    terminal
        .draw(|frame| assert!(ui::render(frame, &picker, &mut state, true)))
        .unwrap();
    assert!(text(terminal.backend().buffer()).contains("No matching browsers."));
}

#[test]
fn render_disables_confirmation_just_below_minimum_and_at_tiny_sizes() {
    let mut picker = Picker::new(vec![browser("a", "Alpha", "")], CurrentDefaults::default());
    for (width, height) in [(31, 10), (32, 9), (20, 5), (1, 1)] {
        for no_color in [false, true] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    let can_select =
                        ui::render(frame, &picker, &mut ListState::default(), no_color);
                    assert!(!can_select);
                    assert_eq!(
                        picker.handle_key(key(KeyCode::Enter), can_select),
                        PickerAction::Continue
                    );
                    assert_eq!(
                        picker.handle_key(key(KeyCode::Esc), can_select),
                        PickerAction::Cancel
                    );
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_outer_inset(buffer);
            assert_selection_background(buffer, None, no_color);
            assert!(!text(buffer).contains("Search"));
            assert!(!text(buffer).contains("Browsers"));
            if width > 1 && height > 1 {
                assert_text_color(
                    buffer,
                    1,
                    1,
                    "Resize",
                    if no_color {
                        Color::Reset
                    } else {
                        Color::DarkGray
                    },
                );
            }
        }
    }
}

#[test]
fn status_is_sanitized_inline_and_preserves_help_inset_and_selection() {
    let mut picker = Picker::new(vec![browser("a", "Alpha", "")], CurrentDefaults::default());
    picker.set_status("Could not set Alpha:\n\u{1b}[31m native\t\u{9b}failure");
    assert_eq!(
        picker.status(),
        "Could not set Alpha:  [31m native  failure"
    );
    for no_color in [false, true] {
        for (width, height) in [(100, 20), (32, 10)] {
            let buffer = render_buffer_at_size(&picker, no_color, width, height);
            assert_outer_inset(&buffer);
            assert_text_color(&buffer, 1, height - 3, "Could not set Alpha:", Color::Reset);
            assert_text_color(
                &buffer,
                1,
                height - 2,
                "↑/↓ move",
                if no_color {
                    Color::Reset
                } else {
                    Color::DarkGray
                },
            );
            assert_rail(&buffer, 5, "Alpha", no_color);
            assert_selection_background(&buffer, Some(5), no_color);
            assert!(buffer
                .content
                .iter()
                .all(|cell| cell.symbol().chars().all(|c| !c.is_control())));
            if no_color {
                assert!(buffer
                    .content
                    .iter()
                    .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset));
            }
        }
    }
}

#[test]
fn refresh_keeps_identity_or_selects_first_remaining_match() {
    let mut picker = Picker::new(
        vec![browser("a", "Alpha", ""), browser("b", "Beta", "")],
        CurrentDefaults::default(),
    );
    picker.set_query("a".into());
    picker.handle_key(key(KeyCode::Down), true);
    picker.refresh(
        Some(vec![browser("b", "Beta", ""), browser("a", "Alpha", "")]),
        CurrentDefaults::default(),
    );
    assert_eq!(picker.query(), "a");
    assert_eq!(picker.selected_browser().unwrap().id, "b");
    picker.refresh(
        Some(vec![browser("a", "Alpha", "")]),
        CurrentDefaults::default(),
    );
    assert_eq!(picker.selected_browser().unwrap().id, "a");
    picker.refresh(Some(Vec::new()), CurrentDefaults::default());
    assert!(picker.selected_browser().is_none());
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter), true),
        PickerAction::Continue
    );
    assert_eq!(
        picker.handle_key(key(KeyCode::Esc), true),
        PickerAction::Cancel
    );
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
