use crate::config::{self, LoopConfig, RemoteConfig, StyleConfig};
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Style, Styled, Stylize},
    symbols::border,
    text::Line,
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame,
};
use std::{
    io,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
    sync::mpsc::{self, Sender, Receiver},
};

#[derive(Clone, Debug)]
struct BufferedOutput {
    text: String,
    style: StyleConfig,
}

impl<'a> BufferedOutput {
    fn into_lines(self) -> Vec<Line<'a>> {
        self.text
            .clone()
            .lines()
            .map(|l| Line::from(l.to_owned()).set_style(Into::<Style>::into(self.style.clone())))
            .collect()
    }
}

#[derive(Debug)]
pub struct App {
    config: config::Config,
    exit: bool,
    event_rx: Receiver<Event>,
    event_tx: Sender<Event>,
    buffer: Arc<Mutex<Vec<BufferedOutput>>>,
    step_idx: usize,
    action_idx: usize,
    finished: bool,
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from(" AutoPilot ".bold());
        let instructions = Line::from(vec![
            " Next ".into(),
            "<Left>".blue().bold(),
            " Prev ".into(),
            "<Right>".blue().bold(),
            " Quit ".into(),
            "<Q> ".blue().bold(),
        ]);
        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(border::THICK);

        let counter_text: Vec<Line<'_>> = self
            .buffer
            .lock()
            .unwrap()
            .iter()
            .map(|t| {
                let mut res = t.clone().into_lines();
                res.push(Line::default());
                res
            })
            .flatten()
            .collect();

        Paragraph::new(counter_text).block(block).render(area, buf);
    }
}

impl App {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            config: config::Config::default(),
            exit: false,
            event_rx,
            event_tx,
            buffer: Arc::new(Mutex::new(Vec::new())),
            step_idx: 0,
            action_idx: 0,
            finished: false,
        }
    }

    /// runs the application's main loop until the user quits
    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        config: config::Config,
    ) -> io::Result<()> {
        self.config = config;
        self.buffer.lock().unwrap().push(BufferedOutput {
            text: format!(" ### {} ###", self.config.steps[self.step_idx].name.clone()).into(),
            style: StyleConfig::title(),
        });

        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    /// updates the application's state based on user input
    fn handle_events(&mut self) -> io::Result<()> {
        match event::read().unwrap() {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                match key_event.code {
                    KeyCode::Char('q') => self.exit(),
                    KeyCode::Left => todo!("No previous action"),
                    KeyCode::Right => self.next_action(),
                    _ => {}
                }
            }
            _ => {}
        };
        Ok(())
    }

    fn next_action_idx(&mut self) {
        if self.finished {
            return;
        }
        let step = &self.config.steps[self.step_idx];
        if step.actions.len() == self.action_idx + 1 {
            if self.config.steps.len() == self.step_idx + 1 {
                self.finished = true;
            } else {
                self.step_idx += 1;
                self.action_idx = 0;
            }
        } else {
            self.action_idx += 1;
        }
    }

    fn next_action(&mut self) {
        if self.finished {
            return;
        }
        match self.config.steps[self.step_idx].actions[self.action_idx].clone() {
            config::Action::Message {
                text,
                style,
                speed: _,
            } => {
                self.write_buf(format!(" > {}", text.clone()), style);
            }
            config::Action::Command {
                command,
                hide_output,
                remote,
                r#loop,
            } => {
                let _ = self.run_command(&command, remote, hide_output.unwrap_or(false), r#loop);
            }
        };
        self.next_action_idx();
    }

    fn run_command(
        &mut self,
        command: &String,
        remote: Option<RemoteConfig>,
        hide: bool,
        loop_config: Option<LoopConfig>,
    ) -> Result<()> {
        let mut cmd = command.clone();
        let mut times = 1;
        let mut delay = 0;

        if let Some(r) = remote {
            cmd = format!("ssh -p {} {} '{}'", r.port.unwrap_or(22), r.host, cmd);
        }

        if let Some(loop_config) = loop_config {
            times = loop_config.times;
            delay = loop_config.delay;
        }

        self.write_buf(format!(" $ {}", command), None);
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            for repetition in 0..times {
                let output = Command::new("sh")
                    .arg("-c")
                    .arg(cmd.clone())
                    .output()
                    .context("Failed to execute command");

                let output = output.unwrap();
                if !hide && !output.stdout.is_empty() {
                    buffer.lock().unwrap().last_mut().unwrap().text +=
                        &format!("\n  {}", String::from_utf8_lossy(&output.stdout));
                }
                if delay > 0 && repetition != times - 1 {
                    thread::sleep(Duration::from_millis(delay));
                }
            }
        });

        Ok(())
    }

    fn write_buf(&mut self, text: String, style: Option<StyleConfig>) {
        self.buffer.lock().unwrap().push(BufferedOutput {
            text,
            style: match style {
                Some(s) => s.clone(),
                None => StyleConfig::default(),
            },
        });
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}
