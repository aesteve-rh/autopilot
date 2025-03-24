// SPDX-FileCopyrightText: 2025 Pavel Bar <pbar@redhat.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::config::{RemoteConfig, SudoConfig};
use anyhow::Context;
use ssh2::Session;
use std::{
    env,
    io::{self, Read, Error, ErrorKind},
    net::TcpStream,
    process::Command,
};
use std::borrow::Cow;

#[derive(Clone)]
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

    fn get_host(&self) -> io::Result<String> {
        match self {
            SessionConfiguration::Local() => Ok(whoami::fallible::hostname()?),
            SessionConfiguration::Remote(_, remote_config) => {
                Ok(format!("{}:{}", remote_config.host, remote_config.port.unwrap()))
            },
        }
    }

    fn run_command(&mut self, cmd: String) -> io::Result<(Vec<u8>, Vec<u8>)> {
        match self {
            SessionConfiguration::Local() => {
                CommandSession::run_local_command("sh", cmd)
            }
            SessionConfiguration::Remote(session, _) => {
                CommandSession::run_remote_command(
                    &session,
                    cmd,
                )
            }
        }
    }
}

pub struct CommandSession {
    session_configuration: SessionConfiguration,
    sudo: Option<SudoConfig>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl CommandSession {
    pub(crate) fn new(remote: Option<RemoteConfig>, sudo: Option<SudoConfig>) -> io::Result<Self> {
        Ok(
            Self {
                session_configuration: if let Some(remote_config) = remote {
                    SessionConfiguration::Remote(Self::init_remote_session(&remote_config)?, remote_config)
                } else {
                    SessionConfiguration::Local()
                },
                sudo,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        )
    }

    pub(crate) fn get_prompt(&self) -> io::Result<String> {
        let (user, prompt_char)  = if let Some(sudo_config) = &self.sudo {
            (sudo_config.user.as_ref().unwrap(), '#')
        } else {
            (&self.session_configuration.get_effective_user(), '$')
        };

        Ok(format!("[{}@{}]{}", user, self.session_configuration.get_host()?, prompt_char))
    }

    pub(crate) fn get_stdout(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub(crate) fn get_stderr(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    pub(crate) fn run_command(&mut self, cmd: String) -> io::Result<()> {
        (self.stdout, self.stderr) = self.session_configuration.run_command(self.get_sudo_command(cmd))?;
        Ok(())
    }

    fn run_local_command(shell: &str, cmd: String) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let output = Command::new(shell)
            .arg("-c")
            .arg(cmd)
            .output()
            .context("Failed to execute command")
            // TODO: improve error handling
            .unwrap();

        Ok((output.stdout, output.stderr))
    }

    fn init_remote_session(remote_config: &RemoteConfig) -> io::Result<Session> {
        let addr = format!("{}:{}", remote_config.host, remote_config.port.unwrap());
        let tcp = TcpStream::connect(addr)?;
        let mut session = Session::new()?;
        session.set_tcp_stream(tcp);
        session.handshake()?;

        // TODO: improve error handling
        let password = Self::resolve_env(remote_config.password.as_ref()).unwrap();
        session.userauth_password(&remote_config.user, &password)?;

        if !session.authenticated() {
            return Err(Error::from(ErrorKind::ConnectionRefused));
        }

        Ok(session)
    }

    fn run_remote_command(session: &Session, cmd: String) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let mut channel = session.channel_session()?;
        channel.exec(cmd.as_str())?;

        let mut stdout: Vec<u8> = Vec::new();
        channel.read_to_end(&mut stdout)?;

        let mut stderr: Vec<u8> = Vec::new();
        channel.stderr().read_to_end(&mut stderr)?;

        Ok((stdout, stderr))
    }

    fn resolve_env(password: Option<&String>) -> anyhow::Result<String> {
        let value = password.unwrap();
        if value.starts_with("$env:") {
            let env_var = &value[5..];
            env::var(env_var).with_context(|| format!("Missing environment variable: {}", env_var))
        } else {
            Ok(value.to_string())
        }
    }

    fn get_sudo_command(&self, cmd: String) -> String {
        if let Some(sudo_config) = &self.sudo {
            let user = sudo_config.user.as_ref().unwrap();
            let password = Self::resolve_env(sudo_config.password.as_ref()).unwrap();
            format!("echo {} | sudo -kS -u {} -p '' {}", password, user, cmd)
        } else {
            cmd
        }
    }
}
