//! The place search popup: type a city, pick from the matches with ↑↓ and Enter.

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph},
};

use crate::SearchState;

const MAX_RESULTS_SHOWN: u16 = 10;

fn input_line(s: &SearchState) -> Line<'static> {
    Line::from(vec![
        Span::styled("› ", Style::new().cyan().bold()),
        Span::raw(s.query.clone()),
        Span::styled("█", Style::new().cyan()),
    ])
}

fn status_line(s: &SearchState) -> Line<'static> {
    if s.searching {
        Line::styled("Searching…", Style::new().dark_gray())
    } else if let Some(m) = &s.message {
        Line::styled(m.clone(), Style::new().yellow())
    } else if s.results.is_empty() {
        Line::styled("Type at least two letters, e.g. Herndon, VA or Paris", Style::new().dark_gray())
    } else {
        Line::styled(format!("{} matches", s.results.len()), Style::new().dark_gray())
    }
}

pub fn draw(f: &mut Frame, area: Rect, s: &SearchState) {
    let list_h = (s.results.len() as u16).clamp(1, MAX_RESULTS_SHOWN);
    // Border (2) + input + status + blank + list + blank + key hints.
    let height = (list_h + 7).min(area.height);
    let width = area.width.saturating_sub(4).min(64);
    let [popup] = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center).areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center).areas(popup);
    f.render_widget(Clear, popup);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().cyan())
        .padding(Padding::horizontal(1))
        .title(Span::styled(" Choose a location ", Style::new().bold()))
        .title_bottom(Line::styled(" ↑↓ select · Enter choose · Esc close ", Style::new().dark_gray()).right_aligned());
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let [input, status, _, list] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    f.render_widget(input_line(s), input);
    f.render_widget(status_line(s), status);

    let items: Vec<ListItem> = s.results.iter().map(|m| ListItem::new(m.label.clone())).collect();
    let mut state = ListState::default().with_selected((!s.results.is_empty()).then_some(s.selected));
    let list_widget = List::new(items).highlight_style(Style::new().black().on_cyan().bold()).highlight_symbol("▸ ");
    f.render_stateful_widget(list_widget, list, &mut state);
}

/// For panes only a few rows tall: the input, then the highlighted match.
pub fn draw_inline(f: &mut Frame, area: Rect, s: &SearchState) {
    let mut first = vec![Span::styled(" Location ", Style::new().bold())];
    first.extend(input_line(s).spans);
    let mut lines = vec![Line::from(first)];
    match s.results.get(s.selected) {
        Some(m) => lines.push(Line::from(vec![
            Span::styled(" ▸ ", Style::new().cyan()),
            Span::raw(m.label.clone()),
            Span::styled(format!("  ({}/{}) ↑↓ Enter", s.selected + 1, s.results.len()), Style::new().dark_gray()),
        ])),
        None => {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(status_line(s).spans);
            lines.push(Line::from(spans).style(status_line(s).style));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}
