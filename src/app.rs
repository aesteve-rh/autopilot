// SPDX-FileCopyrightText: 2025 Albert Esteve <aesteve@redhat.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::{self, CommandType, LoopConfig, RemoteConfig, StyleConfig};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    style::{Style, Styled},
    text::Line,
};
use ssh2::Session;
use std::{
    env, error,
    io::{self, Read},
    net::TcpStream,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use whoami;

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

#[derive(Clone, Debug, Default, PartialEq)]
enum ActionStatus {
    Running,
    Forced,
    #[default]
    Stopped,
}

#[derive(Debug)]
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

    fn write_title(&mut self) {
        self.buffer.lock().unwrap().clear();
        self.buffer.lock().unwrap().push(BufferedOutput {
            text: format!(
                "### {} ###",
                self.config.stages[self.stage_idx].name.clone()
            )
            .into(),
            style: StyleConfig::title(),
        });
    }

    /// updates the application's state based on user input
    pub fn handle_events(&mut self, key_event: KeyEvent) -> io::Result<()> {
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
        let stage = &self.config.stages[self.stage_idx];
        if stage.actions.len() == self.action_idx + 1 {
            if self.config.stages.len() == self.stage_idx + 1 {
                self.finished = true;
            } else {
                self.stage_idx += 1;
                self.action_idx = 0;
            }
        } else {
            self.action_idx += 1;
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

    fn next_action(&mut self) -> io::Result<()> {
        if self.finished && *self.action_status.lock().unwrap() == ActionStatus::Forced {
            return Ok(());
        }
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
                self.write_message(text, style, speed);
            }
            config::Action::Command {
                command,
                sudo,
                hide_output,
                remote,
                r#loop,
            } => {
                if remote.is_some() {
                    self.run_remote_command(
                        &command,
                        remote.unwrap(),
                        sudo.unwrap_or(false),
                        hide_output.unwrap_or(false),
                        r#loop,
                    )
                } else {
                    self.run_command(&command, hide_output.unwrap_or(false), r#loop)
                }?;
            }
        };
        self.next_action_idx();

        Ok(())
    }

    fn write_message(&mut self, text: String, style: Option<StyleConfig>, speed: Option<u64>) {
        let running = self.action_status.clone();
        *running.lock().unwrap() = ActionStatus::Running;
        self.write_buf(String::from("> "), style);
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            for (idx, c) in text.chars().enumerate() {
                if *running.lock().unwrap() == ActionStatus::Forced {
                    buffer.lock().unwrap().last_mut().unwrap().text += &text[idx..text.len()];
                    *running.lock().unwrap() = ActionStatus::Stopped;
                    return;
                }
                buffer.lock().unwrap().last_mut().unwrap().text.push(c);
                thread::sleep(Duration::from_millis(speed.unwrap_or(50)));
            }
            *running.lock().unwrap() = ActionStatus::Stopped;
        });
    }

    fn run_command(
        &mut self,
        command: &CommandType,
        hide: bool,
        loop_config: Option<LoopConfig>,
    ) -> io::Result<()> {
        fn run_single(cmd: String, hide: bool, buffer: Arc<Mutex<Vec<BufferedOutput>>>) {
            let output = Command::new("sh")
                .arg("-c")
                .arg(cmd.clone())
                .output()
                .context("Failed to execute command");

            let output = output.unwrap();
            if !hide && !output.stdout.is_empty() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                buffer
                    .lock()
                    .unwrap()
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&stdout);
            }
            if !output.stderr.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                buffer
                    .lock()
                    .unwrap()
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&stderr);
            }
        }

        self.write_buf(format!("$ {:?}\n", command), None);
        let cmd = command.clone();
        let mut times = 1;
        let mut delay = 0;

        if let Some(loop_config) = loop_config {
            times = loop_config.times;
            delay = loop_config.delay;
        }

        let running = self.action_status.clone();
        *running.lock().unwrap() = ActionStatus::Running;
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            for repetition in 0..times {
                match cmd {
                    CommandType::Single(ref cmd) => {
                        run_single(cmd.clone(), hide, buffer.clone());
                    }
                    CommandType::Multiple(ref cmds) => {
                        for cmd in cmds {
                            run_single(cmd.clone(), hide, buffer.clone());
                        }
                    }
                }
                if delay > 0 && repetition != times - 1 {
                    thread::sleep(Duration::from_millis(delay));
                }
            }
            *running.lock().unwrap() = ActionStatus::Stopped;
        });

        Ok(())
    }

    fn run_remote_command(
        &mut self,
        command: &CommandType,
        remote: RemoteConfig,
        sudo: bool,
        hide: bool,
        loop_config: Option<LoopConfig>,
    ) -> io::Result<()> {
        fn run_single(
            cmd: String,
            session: Session,
            password: String,
            sudo: bool,
            hide: bool,
            buffer: Arc<Mutex<Vec<BufferedOutput>>>,
        ) {
            let cmd = if sudo {
                format!("echo {} | sudo -kS {}", password, cmd)
            } else {
                cmd
            };
            let mut channel = session.channel_session().unwrap();
            channel.exec(cmd.as_str()).unwrap();

            let mut stdout = String::new();
            channel.read_to_string(&mut stdout).unwrap();
            if !hide && !stdout.is_empty() {
                buffer
                    .lock()
                    .unwrap()
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&stdout);
            }
            let mut stderr = String::new();
            channel.stderr().read_to_string(&mut stderr).unwrap();
            if !stderr.is_empty() {
                buffer
                    .lock()
                    .unwrap()
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&stderr);
            }
        }
        let addr = format!("{}:{}", remote.host, remote.port.unwrap_or(22));
        self.write_buf(format!("[{}]$ {:?}\n", addr, command), None);

        let cmd = command.clone();
        let mut times = 1;
        let mut delay = 0;

        if let Some(loop_config) = loop_config {
            times = loop_config.times;
            delay = loop_config.delay;
        }

        let running = self.action_status.clone();
        *running.lock().unwrap() = ActionStatus::Running;
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            let tcp = TcpStream::connect(addr).unwrap();
            let mut sess = Session::new().unwrap();
            sess.set_tcp_stream(tcp);
            sess.handshake().unwrap();

            let user = Self::resolve_env(&remote.user.unwrap_or(whoami::username())).unwrap();
            let password = Self::resolve_env(&remote.password.unwrap_or(String::new())).unwrap();
            sess.userauth_password(&user, &password).unwrap();

            if !sess.authenticated() {
                return;
            }
            for repetition in 0..times {
                match cmd {
                    CommandType::Single(ref cmd) => {
                        run_single(
                            cmd.clone(),
                            sess.clone(),
                            password.clone(),
                            sudo,
                            hide,
                            buffer.clone(),
                        );
                    }
                    CommandType::Multiple(ref cmds) => {
                        for cmd in cmds {
                            run_single(
                                cmd.clone(),
                                sess.clone(),
                                password.clone(),
                                sudo,
                                hide,
                                buffer.clone(),
                            );
                        }
                    }
                }
                if delay > 0 && repetition != times - 1 {
                    thread::sleep(Duration::from_millis(delay));
                }
            }
            *running.lock().unwrap() = ActionStatus::Stopped;
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

    fn resolve_env(value: &str) -> Result<String> {
        if value.starts_with("$env:") {
            let env_var = &value[5..];
            env::var(env_var).with_context(|| format!("Missing environment variable: {}", env_var))
        } else {
            Ok(value.to_string())
        }
    }

    fn exit(&mut self) {
        self.running = false;
    }
}
