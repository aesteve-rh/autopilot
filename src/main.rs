mod config;
mod widget;

use color_eyre;
use std::{io, path::Path};

fn main() -> io::Result<()> {
    color_eyre::install().unwrap();
    let config = config::Config::load_config(Path::new("./examples/basic.yaml"))
        .expect("Parsing configuration failed");
    let mut terminal = ratatui::init();
    let result = widget::App::new().run(&mut terminal, config);
    ratatui::restore();
    result
}
