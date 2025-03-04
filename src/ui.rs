use ratatui::{
    style::{Color, Style, Stylize},
    symbols::border,
    text::Line,
    widgets::{Block, Paragraph},
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
}

/// Renders the user interface widgets.
pub fn render(app: &mut App, frame: &mut Frame) {
    // This is where you add new widgets.
    // See the following resources:
    // - https://docs.rs/ratatui/latest/ratatui/widgets/index.html
    // - https://github.com/ratatui/ratatui/tree/master/examples
    frame.render_widget(
        Paragraph::new(render_text(app))
            .block(render_block())
            .style(Style::default().fg(Color::Cyan).bg(Color::Black)),
        frame.area(),
    )
}
