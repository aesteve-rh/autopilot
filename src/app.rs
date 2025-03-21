// SPDX-FileCopyrightText: 2025 Albert Esteve <aesteve@redhat.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::{self, CommandType, LoopConfig, RemoteConfig, StyleConfig, SudoConfig};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    style::{Color, Style, Styled},
    text::{Line, Span},
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

    fn next_action(&mut self) -> io::Result<()> {
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
                remote,
                r#loop,
            } => {
                if let Some(remote_config) = remote {
                    self.run_remote_command(
                        &command,
                        remote_config,
                        sudo,
                        hide_stdout.unwrap(),
                        hide_stderr.unwrap(),
                        r#loop.unwrap(),
                    )
                } else {
                    self.run_local_command(
                        &command,
                        sudo,
                        hide_stdout.unwrap(),
                        hide_stderr.unwrap(),
                        r#loop.unwrap(),
                    )
                }?;
            }
        };
        self.next_action_idx();

        Ok(())
    }

    fn write_message(&mut self, text: String, style: Option<StyleConfig>, speed: u64) {
        let running = self.action_status.clone();
        *running.lock().unwrap() = ActionStatus::Running;
        self.write_buf(String::from("> "), style);
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            for (idx, c) in text.chars().enumerate() {
                if running.lock().unwrap().force_stop() {
                    // Print the rest of the string all at once.
                    Self::add_to_buf(buffer, &text[idx..text.len()], false);
                    break;
                }
                buffer.lock().unwrap().last_mut().unwrap().text.push(c);
                thread::sleep(Duration::from_millis(speed));
            }
            *running.lock().unwrap() = ActionStatus::Stopped;
        });
    }

    fn run_local_command(
        &mut self,
        command: &CommandType,
        sudo: Option<SudoConfig>,
        hide_stdout: bool,
        hide_stderr: bool,
        loop_config: LoopConfig,
    ) -> io::Result<()> {

        let prompt = Self::get_prompt(
            sudo.clone(),
            &whoami::username(),
            &whoami::fallible::hostname()?);
        let cmd = command.get_command();
        self.write_buf(format!("{} {}\n", prompt, cmd), None);

        let cmd = Self::get_sudo_command(cmd, sudo);
        let running = self.action_status.clone();
        *running.lock().unwrap() = ActionStatus::Running;
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            let times = loop_config.times;
            let delay = loop_config.delay.unwrap();
            for repetition in 0..times {
                if running.lock().unwrap().force_stop() {
                    Self::add_to_buf(buffer, "Command interrupted!\n", hide_stdout);
                    break;
                }

                let output = Command::new("sh")
                    .arg("-c")
                    .arg(cmd.clone())
                    .output()
                    .context("Failed to execute command")
                    .unwrap();

                Self::add_to_buf(buffer.clone(), &String::from_utf8_lossy(&output.stdout), hide_stdout);
                Self::add_to_buf(buffer.clone(), &String::from_utf8_lossy(&output.stderr), hide_stderr);

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
        remote_config: RemoteConfig,
        sudo: Option<SudoConfig>,
        hide_stdout: bool,
        hide_stderr: bool,
        loop_config: LoopConfig,
    ) -> io::Result<()> {
        let addr = format!("{}:{}", remote_config.host, remote_config.port.unwrap());
        let prompt = Self::get_prompt(
            sudo.clone(),
            &remote_config.user,
            &addr);
        let cmd = command.get_command();
        self.write_buf(format!("{} {}\n", prompt, cmd), None);

        let cmd = Self::get_sudo_command(cmd, sudo);
        let running = self.action_status.clone();
        *running.lock().unwrap() = ActionStatus::Running;
        let buffer = self.buffer.clone();
        thread::spawn(move || {
            let tcp = TcpStream::connect(addr).unwrap();
            let mut session = Session::new().unwrap();
            session.set_tcp_stream(tcp);
            session.handshake().unwrap();

            let password = Self::resolve_env(&remote_config.password.unwrap()).unwrap();
            session.userauth_password(&remote_config.user, &password).unwrap();

            if !session.authenticated() {
                return;
            }

            let times = loop_config.times;
            let delay = loop_config.delay.unwrap();
            for repetition in 0..times {
                if running.lock().unwrap().force_stop() {
                    Self::add_to_buf(buffer, "Command interrupted!\n", hide_stdout);
                    break;
                }

                let mut channel = session.channel_session().unwrap();
                channel.exec(cmd.as_str()).unwrap();

                let mut stdout = String::new();
                channel.read_to_string(&mut stdout).unwrap();
                Self::add_to_buf(buffer.clone(), &stdout, hide_stdout);

                let mut stderr = String::new();
                channel.stderr().read_to_string(&mut stderr).unwrap();
                Self::add_to_buf(buffer.clone(), &stderr, hide_stderr);

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

    fn get_prompt(sudo: Option<SudoConfig>, effective_user: &String, host: &String) -> String {
        let (user, prompt_char)  = if let Some(sudo_config) = sudo {
            (&sudo_config.user.unwrap(), '#')
        } else {
            (effective_user, '$')
        };

        format!("[{}@{}]{}", user, host, prompt_char)
    }

    fn get_sudo_command(cmd: String, sudo: Option<SudoConfig>) -> String {
        if let Some(sudo_config) = sudo {
            let user = sudo_config.user.unwrap();
            let password = Self::resolve_env(&sudo_config.password.unwrap()).unwrap();
            format!("echo {} | sudo -kS -u {} -p '' {}", password, user, cmd)
        } else {
            cmd
        }
    }
}
