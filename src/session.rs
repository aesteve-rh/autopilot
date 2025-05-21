// SPDX-FileCopyrightText: 2025 Pavel Bar <pbar@redhat.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::{CommandType, RemoteConfig, SudoConfig};
use anyhow::{ensure, Context, Result};
use ssh2::Session;
use std::borrow::Cow;
use std::{
    env,
    io::Read,
    net::TcpStream,
    process::Command,
};

enum SessionConfiguration {
    Local(),
    Remote(Session, RemoteConfig),
}

impl SessionConfiguration {
    fn get_effective_user(&self) -> String {
        match self {
            SessionConfiguration::Local() => whoami::username(),
            SessionConfiguration::Remote(_, remote_config) => remote_config.user.clone(),
        }
    }

    fn get_host(&self) -> Result<String> {
        match self {
            SessionConfiguration::Local() => Ok(whoami::fallible::hostname()?),
            SessionConfiguration::Remote(_, remote_config) => {
                Ok(format!("{}:{}", remote_config.host, remote_config.port.unwrap()))
            },
        }
    }
}

pub struct CommandSession {
    command: String,
    session_configuration: SessionConfiguration,
    sudo: Option<SudoConfig>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl CommandSession {
    pub(crate) fn new(
        command: &CommandType,
        remote: Option<RemoteConfig>,
        sudo: Option<SudoConfig>,
    ) -> Result<Self> {
        Ok(
            Self {
                command: Self::resolve_command(command)?,
                session_configuration: if let Some(remote_config) = remote {
                    Self::init_remote_session(Self::resolve_remote_config(remote_config)?)?
                } else {
                    SessionConfiguration::Local()
                },
                sudo: sudo
                    .map(|sudo_config| Self::resolve_sudo_config(sudo_config))
                    .transpose()?,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        )
    }

    pub(crate) fn get_prompt(&self) -> Result<String> {
        let (user, prompt_char)  = if let Some(sudo_config) = &self.sudo {
            (sudo_config.user.as_ref().unwrap(), '#')
        } else {
            (&self.session_configuration.get_effective_user(), '$')
        };

        Ok(
            format!(
                "[{}@{}]{} {}\n",
                user,
                self.session_configuration.get_host()?,
                prompt_char,
                self.command,
            )
        )
    }

    pub(crate) fn get_stdout(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub(crate) fn get_stderr(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    pub(crate) fn run_command(&mut self) -> Result<()> {
        let cmd = self.get_sudo_command();
        (self.stdout, self.stderr) = match &self.session_configuration {
            SessionConfiguration::Local() => {
                Self::run_local_command("sh", cmd)?
            }
            SessionConfiguration::Remote(session, _) => {
                Self::run_remote_command(session, cmd)?
            }
        };

        Ok(())
    }

    fn run_local_command(shell: &str, cmd: String) -> Result<(Vec<u8>, Vec<u8>)> {
        let output = Command::new(shell)
            .arg("-c")
            .arg(cmd)
            .output()
            .context("Failed to execute a local command")?;

        Ok((output.stdout, output.stderr))
    }

    fn resolve_remote_config(remote_config: RemoteConfig) -> Result<RemoteConfig> {
        Ok(
            RemoteConfig {
                host: Self::resolve_env_str(remote_config.host)?,
                port: remote_config.port,
                user: Self::resolve_env_str(remote_config.user)?,
                password: Self::resolve_env_opt(remote_config.password)?,
            }
        )
    }

    fn resolve_sudo_config(sudo_config: SudoConfig) -> Result<SudoConfig> {
        Ok(
            SudoConfig {
                user: Self::resolve_env_opt(sudo_config.user)?,
                password: Self::resolve_env_opt(sudo_config.password)?,
            }
        )
    }

    fn init_remote_session(remote_config: RemoteConfig) -> Result<SessionConfiguration> {
        let addr = format!("{}:{}", remote_config.host, remote_config.port.unwrap());
        let tcp = TcpStream::connect(addr)?;
        let mut session = Session::new()?;
        session.set_tcp_stream(tcp);
        session.handshake()?;

        session.userauth_password(&remote_config.user, remote_config.password.as_ref().unwrap())?;
        ensure!(session.authenticated(), "Session password authentication failed");

        Ok(SessionConfiguration::Remote(session, remote_config))
    }

    fn run_remote_command(session: &Session, cmd: String) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut channel = session.channel_session()?;
        channel.exec(cmd.as_str())?;

        let mut stdout: Vec<u8> = Vec::new();
        channel.read_to_end(&mut stdout)?;

        let mut stderr: Vec<u8> = Vec::new();
        channel.stderr().read_to_end(&mut stderr)?;

        Ok((stdout, stderr))
    }

    fn resolve_env_str(value: String) -> Result<String> {
        if value.starts_with("$env:") {
            let env_var = &value[5..];
            env::var(env_var).with_context(|| format!("Missing environment variable: '{}'", env_var))
        } else {
            Ok(value)
        }
    }

    fn resolve_env_opt(value_opt: Option<String>) -> Result<Option<String>> {
        value_opt
            .map(|value| Self::resolve_env_str(value))
            .transpose()
    }

    fn resolve_command(command: &CommandType) -> Result<String> {
        let mut cmd_parts_resolved = Vec::new();
        for s in command.get_command().split_whitespace() {
            cmd_parts_resolved.push(Self::resolve_env_str(s.to_string())?);
        }

        Ok(cmd_parts_resolved.join(" "))
    }

    fn get_sudo_command(&self) -> String {
        if let Some(sudo_config) = &self.sudo {
            format!(
                "echo {} | sudo -kS -u {} -p '' {}",
                sudo_config.password.as_ref().unwrap(),
                sudo_config.user.as_ref().unwrap(),
                self.command,
            )
        } else {
            self.command.clone()
        }
    }
}
