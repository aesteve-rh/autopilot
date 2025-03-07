// SPDX-FileCopyrightText: 2025 Albert Esteve <aesteve@redhat.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use ratatui::{
    prelude::Margin,
    style::{Color, Style, Stylize},
    symbols::border,
    text::Line,
    widgets::{Block, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::App;

fn render_text(app: &App) -> Vec<Line<'_>> {
    app.buffer
        .lock()
        .unwrap()
        .iter()
        .map(|t| {
            let mut res = t.clone().into_lines();
            res.push(Line::default());
            res
        })
        .flatten()
        .collect()
}

fn render_block() -> ratatui::widgets::Block<'static> {
    let title = Line::from(" AutoPilot ".bold());
    let instructions = Line::from(vec![
        " Next ".into(),
        "<Left>".blue().bold(),
        " Prev ".into(),
        "<Right>".blue().bold(),
        " Quit ".into(),
        "<Q> ".blue().bold(),
    ]);
    Block::bordered()
        .title(title.centered())
        .title_bottom(instructions.centered())
        .border_set(border::THICK)
        .padding(Padding::horizontal(1))
}

/// Renders the user interface widgets.
pub fn render(app: &mut App, frame: &mut Frame) {
    // This is where you add new widgets.
    // See the following resources:
    // - https://docs.rs/ratatui/latest/ratatui/widgets/index.html
    // - https://github.com/ratatui/ratatui/tree/master/examples
    let area = frame.area();
    let text = render_text(app);
    let total_lines = text.len() as u16;
    let position = total_lines.saturating_sub(app.scroll);
    let vertical_scroll = if position > area.height {
        position.saturating_sub(area.height) + 1
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(render_block())
            .style(Style::default().fg(Color::Gray).bg(Color::Black))
            .scroll((vertical_scroll, 0)),
        area,
    );

    if total_lines > area.height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scrollbar_state =
            ScrollbarState::new(total_lines as usize).position(position as usize);
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}
