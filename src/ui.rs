use std::io::{self, IsTerminal};

use anyhow::{bail, Context, Result};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{
    backend::{Browser, CurrentDefaults},
    display_text,
};

/// Lower scores rank first: leading characters plus gaps, then total length.
/// Matching uses Unicode lowercase scalars, not locale-specific case folding.
pub fn fuzzy_score(query: &str, text: &str) -> Option<(usize, usize)> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let query: Vec<char> = query.to_lowercase().chars().collect();
    if query.is_empty() {
        return Some((0, text.len()));
    }
    let mut cursor = 0;
    for character in &query {
        let offset = text[cursor..]
            .iter()
            .position(|candidate| candidate == character)?;
        cursor += offset + 1;
    }
    Some((cursor - query.len(), text.len()))
}

#[derive(Debug, Eq, PartialEq)]
pub enum PickerAction {
    Continue,
    Cancel,
    Select(Browser),
}

pub struct Picker {
    browsers: Vec<Browser>,
    current: CurrentDefaults,
    query: String,
    visible: Vec<usize>,
    selected: Option<usize>,
}

impl Picker {
    pub fn new(browsers: Vec<Browser>, current: CurrentDefaults) -> Self {
        let visible = (0..browsers.len()).collect();
        let selected = (!browsers.is_empty()).then_some(0);
        Self {
            browsers,
            current,
            query: String::new(),
            visible,
            selected,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn visible_browsers(&self) -> impl Iterator<Item = &Browser> {
        self.visible.iter().map(|&index| &self.browsers[index])
    }

    pub fn selected_browser(&self) -> Option<&Browser> {
        self.selected
            .map(|index| &self.browsers[self.visible[index]])
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        let mut ranked = Vec::new();
        for (index, browser) in self.browsers.iter().enumerate() {
            let score = [&browser.name, &browser.id, &browser.detail]
                .iter()
                .enumerate()
                .filter_map(|(field, text)| {
                    fuzzy_score(&self.query, text).map(|(gaps, length)| (gaps, field, length))
                })
                .min();
            if let Some(score) = score {
                ranked.push((score, index));
            }
        }
        if !self.query.is_empty() {
            ranked.sort_by_key(|&(score, index)| (score, index));
        }
        self.visible = ranked.into_iter().map(|(_, index)| index).collect();
        self.selected = (!self.visible.is_empty()).then_some(0);
    }

    pub fn handle_key(&mut self, key: KeyEvent, can_select: bool) -> PickerAction {
        if key.kind == KeyEventKind::Release {
            return PickerAction::Continue;
        }
        if key.code == KeyCode::Esc
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'C')))
        {
            return PickerAction::Cancel;
        }
        match key.code {
            KeyCode::Enter if can_select => {
                if let Some(browser) = self.selected_browser() {
                    return PickerAction::Select(browser.clone());
                }
            }
            KeyCode::Up => {
                if let Some(index) = self.selected {
                    self.selected = Some(if index == 0 {
                        self.visible.len() - 1
                    } else {
                        index - 1
                    });
                }
            }
            KeyCode::Down => {
                if let Some(index) = self.selected {
                    self.selected = Some((index + 1) % self.visible.len());
                }
            }
            KeyCode::Backspace => {
                let mut query = self.query.clone();
                query.pop();
                self.set_query(query);
            }
            KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.set_query(String::new())
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !character.is_control() =>
            {
                let mut query = self.query.clone();
                query.push(character);
                self.set_query(query);
            }
            _ => {}
        }
        PickerAction::Continue
    }
}

pub fn current_marker(browser: &Browser, current: &CurrentDefaults) -> &'static str {
    match (
        current.http.as_deref() == Some(&browser.id),
        current.https.as_deref() == Some(&browser.id),
    ) {
        (true, true) => "[HTTP HTTPS]",
        (true, false) => "[HTTP]",
        (false, true) => "[HTTPS]",
        (false, false) => "",
    }
}

fn palette_style(no_color: bool, color: Color) -> Style {
    if no_color {
        Style::default()
    } else {
        Style::default().fg(color)
    }
}

pub fn selection_style(no_color: bool) -> Style {
    if no_color {
        Style::default()
    } else {
        // Only shade the row: a foreground here would override the current tag.
        Style::default().bg(Color::DarkGray)
    }
}

pub fn usable_size(area: Rect) -> bool {
    area.width >= 32 && area.height >= 10
}

/// Returns whether selection is visible and safe to confirm at this size.
pub fn render(frame: &mut Frame, picker: &Picker, state: &mut ListState, no_color: bool) -> bool {
    let outer = frame.area();
    let area = outer.inner(Margin::new(1, 1));
    let subdued_style = palette_style(no_color, Color::DarkGray);
    if !usable_size(outer) {
        frame.render_widget(
            Paragraph::new("Resize to at least 32x10. Esc cancels.").style(subdued_style),
            area,
        );
        return false;
    }
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);
    let search = Paragraph::new(display_text(&picker.query))
        .style(Style::default())
        .block(
            Block::bordered()
                .border_style(subdued_style)
                .title_style(Style::default().fg(Color::Reset))
                .title("Search"),
        );
    frame.render_widget(search, areas[0]);
    let block = Block::bordered()
        .border_style(subdued_style)
        .title_style(Style::default().fg(Color::Reset))
        .title(format!("Browsers ({})", picker.visible.len()));
    if picker.visible.is_empty() {
        let message = if picker.browsers.is_empty() {
            "No browsers found."
        } else {
            "No matching browsers. Ctrl-U clears."
        };
        frame.render_widget(
            Paragraph::new(message).style(subdued_style).block(block),
            areas[1],
        );
    } else {
        let current_style = palette_style(no_color, Color::Yellow).add_modifier(Modifier::BOLD);
        let items = picker
            .visible_browsers()
            .enumerate()
            .map(|(index, browser)| {
                let name = display_text(&browser.name);
                let mut spans = vec![Span::raw(name.clone())];
                if !current_marker(browser, &picker.current).is_empty() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled("[current]", current_style));
                }
                // Details only disambiguate namesakes, including ones hidden by the query.
                let namesakes: Vec<_> = picker
                    .browsers
                    .iter()
                    .filter(|other| display_text(&other.name) == name)
                    .collect();
                if namesakes.len() > 1 {
                    let detail = if browser.detail.is_empty()
                        || namesakes
                            .iter()
                            .filter(|other| other.detail == browser.detail)
                            .count()
                            > 1
                    {
                        &browser.id
                    } else {
                        &browser.detail
                    };
                    // Gray details would disappear against the selected row's gray background.
                    let detail_style = if picker.selected == Some(index) {
                        Style::default().fg(Color::Reset)
                    } else {
                        subdued_style
                    };
                    spans.push(Span::styled(
                        format!(" — {}", display_text(detail)),
                        detail_style.add_modifier(Modifier::DIM),
                    ));
                }
                ListItem::new(Line::from(spans))
            });
        state.select(picker.selected);
        frame.render_stateful_widget(
            List::new(items)
                .style(Style::default())
                .block(block)
                .highlight_symbol("> ")
                .highlight_style(selection_style(no_color)),
            areas[1],
            state,
        );
    }
    frame.render_widget(
        Paragraph::new("↑/↓ move · Enter set · Esc cancel · Ctrl-U clear").style(subdued_style),
        areas[2],
    );
    true
}

struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn restore(&mut self) -> Result<()> {
        // Attempt both restorations even if one fails; a setter must not run on failure.
        let raw = disable_raw_mode();
        let screen = execute!(io::stdout(), LeaveAlternateScreen, Show);
        raw.context("Failed to leave terminal raw mode")?;
        screen.context("Failed to restore terminal screen")?;
        self.active = false;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

pub fn pick(browsers: Vec<Browser>, current: CurrentDefaults) -> Result<Option<Browser>> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        bail!("The picker needs a terminal. Use `defbrow list` or `defbrow set <id>` instead.");
    }
    let mut guard = TerminalGuard { active: true };
    // ratatui also installs a hook that restores the terminal before printing a panic.
    let mut terminal = ratatui::try_init().context("Failed to initialize terminal")?;
    let mut picker = Picker::new(browsers, current);
    let mut state = ListState::default();
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let result = loop {
        let mut can_select = false;
        terminal.draw(|frame| can_select = render(frame, &picker, &mut state, no_color))?;
        if let Event::Key(key) = event::read()? {
            match picker.handle_key(key, can_select) {
                PickerAction::Continue => {}
                PickerAction::Cancel => break None,
                PickerAction::Select(browser) => break Some(browser),
            }
        }
    };
    guard.restore()?;
    Ok(result)
}
