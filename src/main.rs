mod app;
mod config;
mod event;
mod tui;
mod ui;

use crate::{
    app::{App, AppResult},
    event::{Event, EventHandler},
    tui::Tui,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, path::Path};

#[tokio::main]
async fn main() -> AppResult<()> {
    let config = config::Config::load_config(Path::new("./examples/basic.yaml"))
        .expect("Parsing configuration failed");
    // Create an application.
    let mut app = App::new(config);

    // Initialize the terminal user interface.
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    let events = EventHandler::new(50);
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    // Start the main loop.
    while app.running {
        // Render the user interface.
        tui.draw(&mut app)?;
        // Handle events.
        match tui.events.next().await? {
            Event::Tick => app.tick(),
            Event::Key(key_event) => app.handle_events(key_event)?,
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
        }
    }

    // Exit the user interface.
    tui.exit()?;
    Ok(())
}
