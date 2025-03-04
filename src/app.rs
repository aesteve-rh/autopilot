use crate::config::{self, LoopConfig, RemoteConfig, StyleConfig};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    style::{Style, Styled},
    text::Line,
};
use std::{
    error, io,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

/// Application result type.
pub type AppResult<T> = std::result::Result<T, Box<dyn error::Error>>;

#[derive(Clone, Debug)]
pub struct BufferedOutput {
    text: String,
    style: StyleConfig,
}

impl<'a> BufferedOutput {
    pub fn into_lines(self) -> Vec<Line<'a>> {
        self.text
            .clone()
            .lines()
            .map(|l| Line::from(l.to_owned()).set_style(Into::<Style>::into(self.style.clone())))
            .collect()
    }
}

#[derive(Debug)]
pub struct App {
    /// Is the application running?
    pub running: bool,
    config: config::Config,
    pub buffer: Arc<Mutex<Vec<BufferedOutput>>>,
    step_idx: usize,
    action_idx: usize,
    action_run: Arc<Mutex<bool>>,
    finished: bool,
}

impl App {
    pub fn new(config: config::Config) -> Self {
        let app = Self {
            running: true,
            config,
            buffer: Arc::new(Mutex::new(Vec::new())),
            step_idx: 0,
            action_idx: 0,
            action_run: Arc::new(Mutex::new(false)),
            finished: false,
        };
        app.buffer.lock().unwrap().push(BufferedOutput {
            text: format!(" ### {} ###", app.config.steps[app.step_idx].name.clone()).into(),
            style: StyleConfig::title(),
        });
        app
    }

    /// updates the application's state based on user input
    pub fn handle_events(&mut self, key_event: KeyEvent) -> io::Result<()> {
        Ok(match key_event.code {
            KeyCode::Char('q') => self.exit(),
            KeyCode::Left => todo!("No previous action"),
            KeyCode::Right => self.next_action(),
            _ => {}
        })
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&self) {}

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
        if self.finished || *self.action_run.lock().unwrap() {
            return;
        }
        match self.config.steps[self.step_idx].actions[self.action_idx].clone() {
            config::Action::Message { text, style, speed } => {
                self.write_message(text, style, speed);
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

    fn write_message(&mut self, text: String, style: Option<StyleConfig>, speed: Option<u64>) {
        let running = self.action_run.clone();
        *running.lock().unwrap() = true;
        self.write_buf(String::from(" > "), style);
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            for c in text.chars() {
                buffer.lock().unwrap().last_mut().unwrap().text.push(c);
                thread::sleep(Duration::from_millis(speed.unwrap_or(50)));
            }
            *running.lock().unwrap() = false;
        });
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

        let running = self.action_run.clone();
        *running.lock().unwrap() = true;
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
            *running.lock().unwrap() = false;
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
        self.running = false;
    }
}
