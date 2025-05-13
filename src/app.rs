// SPDX-FileCopyrightText: 2025 Albert Esteve <aesteve@redhat.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::{self, CommandType, LoopConfig, RemoteConfig, StyleConfig, SudoConfig};
use crate::session::CommandSession;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    style::{Color, Style, Styled},
    text::{Line, Span},
};
use std::{
    error,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

/// Application result type.
pub type AppResult<T> = Result<T, Box<dyn error::Error>>;

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

#[derive(Clone, Debug, Default, PartialEq)]
enum ActionStatus {
    Running,
    Forced,
    #[default]
    Stopped,
}

impl ActionStatus {
    pub fn force_stop(&self) -> bool {
        *self == ActionStatus::Forced
    }
}

pub struct App {
    /// Is the application running?
    pub running: bool,
    config: config::Config,
    pub buffer: Arc<Mutex<Vec<BufferedOutput>>>,
    stage_idx: usize,
    action_idx: usize,
    action_status: Arc<Mutex<ActionStatus>>,
    pub scroll: u16,
    finished: bool,
}

impl App {
    pub fn new(config: config::Config) -> Self {
        let mut app = Self {
            running: true,
            config,
            buffer: Arc::new(Mutex::new(Vec::new())),
            stage_idx: 0,
            action_idx: 0,
            action_status: Arc::new(Mutex::new(ActionStatus::default())),
            scroll: 0,
            finished: false,
        };
        app.write_title();
        app
    }

    pub fn status(&self) -> Span<'static> {
        match *self.action_status.lock().unwrap() {
            ActionStatus::Forced | ActionStatus::Stopped if self.finished => {
                Span::styled(" [ Finished ] ", Style::default().fg(Color::LightYellow))
            }
            ActionStatus::Running => {
                Span::styled(" ◄ Running... ▶ ", Style::default().fg(Color::LightGreen))
            }
            ActionStatus::Forced => {
                Span::styled(" ■ Stopping... ■ ", Style::default().fg(Color::Red))
            }
            ActionStatus::Stopped => {
                Span::styled(" ■ Stopped ■ ", Style::default().fg(Color::LightRed))
            }
        }
    }

    fn write_title(&mut self) {
        self.buffer.lock().unwrap().clear();
        self.buffer.lock().unwrap().push(BufferedOutput {
            text: format!(
                "### {} ###",
                self.config.stages[self.stage_idx].name
            )
            .into(),
            style: StyleConfig::title(),
        });
    }

    /// updates the application's state based on user input
    pub fn handle_events(&mut self, key_event: KeyEvent) -> Result<()> {
        Ok(match key_event.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.exit(),
            KeyCode::Left => self.prev_action(),
            KeyCode::Right => self.next_action()?,
            KeyCode::Up => self.scroll_up(1),
            KeyCode::PageUp => self.scroll_up(10),
            KeyCode::Down => self.scroll_down(1),
            KeyCode::PageDown => self.scroll_down(10),
            _ => {}
        })
    }

    fn scroll_up(&mut self, value: u16) {
        self.scroll = self.scroll.saturating_add(value);
    }

    fn scroll_down(&mut self, value: u16) {
        self.scroll = self.scroll.saturating_sub(value);
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&self) {}

    fn next_action_idx(&mut self) {
        if self.finished {
            return;
        }
        self.action_idx += 1;

        let stage = &self.config.stages[self.stage_idx];
        if stage.actions.len() == self.action_idx {
            if self.config.stages.len() == self.stage_idx + 1 {
                self.finished = true;
            } else {
                self.stage_idx += 1;
                self.action_idx = 0;
            }
        }
    }

    fn prev_action(&mut self) {
        if *self.action_status.lock().unwrap() != ActionStatus::Stopped {
            return;
        }
        if self.finished {
            self.finished = false;
        }
        if self.action_idx == 0 {
            if self.stage_idx > 0 {
                self.stage_idx -= 1;
                self.write_title();
            }
            return;
        }

        self.action_idx -= 1;
        self.buffer.lock().unwrap().pop();
    }

    fn next_action(&mut self) -> Result<()> {
        if *self.action_status.lock().unwrap() == ActionStatus::Running {
            *self.action_status.lock().unwrap() = ActionStatus::Forced;
            return Ok(());
        }
        if self.finished {
            return Ok(());
        }
        if self.action_idx == 0 && self.stage_idx > 0 {
            self.write_title();
        }
        match self.config.stages[self.stage_idx].actions[self.action_idx].clone() {
            config::Action::Message { text, style, speed } => {
                self.write_message(text, style, speed.unwrap());
            }
            config::Action::Command {
                command,
                sudo,
                hide_stdout,
                hide_stderr,
                style,
                remote,
                r#loop,
            } => {
                self.run_command(
                    command,
                    remote,
                    sudo,
                    hide_stdout.unwrap(),
                    hide_stderr.unwrap(),
                    style,
                    r#loop.unwrap(),
                )?;
            }
        };
        self.next_action_idx();

        Ok(())
    }

    fn write_message(&mut self, text: String, style: Option<StyleConfig>, speed: u64) {
        let exec_status = self.action_status.clone();
        *exec_status.lock().unwrap() = ActionStatus::Running;

        self.write_buf(String::from("> "), style);
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            for (idx, c) in text.chars().enumerate() {
                if exec_status.lock().unwrap().force_stop() {
                    // Print the rest of the string all at once.
                    Self::add_to_buf(buffer, &text[idx..text.len()], false);
                    break;
                }
                buffer.lock().unwrap().last_mut().unwrap().text.push(c);
                thread::sleep(Duration::from_millis(speed));
            }
            *exec_status.lock().unwrap() = ActionStatus::Stopped;
        });
    }

    fn run_command(
        &mut self,
        command: CommandType,
        remote: Option<RemoteConfig>,
        sudo: Option<SudoConfig>,
        hide_stdout: bool,
        hide_stderr: bool,
        style: Option<StyleConfig>,
        loop_config: LoopConfig,
    ) -> Result<()> {
        let exec_status = self.action_status.clone();
        *exec_status.lock().unwrap() = ActionStatus::Running;

        let mut command_session = match CommandSession::new(remote, sudo) {
            Ok(command_session) => command_session,
            Err(e) => {
                self.write_buf(
                    format!(
                        "Failed to initialize a new session.\n\tCommand: {}\n\tError:   {}",
                        command.get_command(),
                        e,
                    ),
                    Some(StyleConfig::error()),
                );
                *exec_status.lock().unwrap() = ActionStatus::Stopped;
                return Ok(());
            }
        };

        let prompt = command_session.get_prompt()?;

        let cmd = command.get_command();
        self.write_buf(format!("{} {}\n", prompt, cmd), style);

        let buffer = self.buffer.clone();
        thread::spawn(move || {
            let times = loop_config.times;
            let delay = loop_config.delay.unwrap();
            for repetition in 0..times {
                if exec_status.lock().unwrap().force_stop() {
                    Self::add_to_buf(buffer, "Command interrupted!\n", hide_stdout);
                    break;
                }

                command_session.run_command(cmd.clone()).unwrap();
                Self::add_to_buf(buffer.clone(),
                                 &command_session.get_stdout(),
                                 hide_stdout);
                Self::add_to_buf(buffer.clone(),
                                 &command_session.get_stderr(),
                                 hide_stderr);

                if delay > 0 && repetition != times - 1 {
                    thread::sleep(Duration::from_millis(delay));
                }
            }
            *exec_status.lock().unwrap() = ActionStatus::Stopped;
        });

        Ok(())
    }

    fn write_buf(&mut self, text: String, style: Option<StyleConfig>) {
        self.buffer.lock().unwrap().push(BufferedOutput {
            text,
            style: style.unwrap_or_else(|| StyleConfig::default()),
        });
    }

    fn add_to_buf(buffer: Arc<Mutex<Vec<BufferedOutput>>>, output: &str, hide_output: bool) {
        if !hide_output && !output.is_empty() {
            buffer
                .lock()
                .unwrap()
                .last_mut()
                .unwrap()
                .text
                .push_str(&output);
        }
    }

    fn exit(&mut self) {
        self.running = false;
    }
}
