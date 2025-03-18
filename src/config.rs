// SPDX-FileCopyrightText: 2025 Albert Esteve <aesteve@redhat.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use jsonschema;
use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StyleConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
}

impl StyleConfig {
    pub fn title() -> Self {
        let mut title = StyleConfig::default();
        title.bold = Some(true);
        title.color = Some(String::from("white"));
        title
    }
}

impl Into<Style> for StyleConfig {
    fn into(self) -> Style {
        let mut style = Style::default();

        if let Some(color) = &self.color {
            style = style.fg(match color.as_str() {
                "green" => Color::Green,
                "yellow" => Color::Yellow,
                "blue" => Color::Blue,
                "cyan" => Color::Cyan,
                "red" => Color::Red,
                "magenta" => Color::Magenta,
                _ => Color::White,
            });
        }

        if self.bold.unwrap_or(false) {
            style = style.add_modifier(Modifier::BOLD);
        }

        if self.italic.unwrap_or(false) {
            style = style.add_modifier(Modifier::ITALIC);
        }

        style
    }
}

#[derive(Deserialize, Serialize)]
pub struct Stage {
    pub name: String,
    pub actions: Vec<Action>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandType {
    Single(String),
    Multiple(Vec<String>),
}

impl CommandType {
    pub fn get_command(&self) -> String {
        match self {
            Self::Single(ref cmd) => {
                cmd.clone()
            }
            Self::Multiple(ref cmds) => {
                cmds.join(" && ")
            }
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    Message {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<StyleConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        speed: Option<u64>,
    },
    Command {
        command: CommandType,
        #[serde(skip_serializing_if = "Option::is_none")]
        sudo: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hide_output: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        remote: Option<RemoteConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#loop: Option<LoopConfig>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RemoteConfig {
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoopConfig {
    pub times: u32,
    pub delay: u64,
}

#[derive(Default, Deserialize, Serialize)]
pub struct Config {
    pub stages: Vec<Stage>,
}

impl Config {
    fn validate_config(config: &Config) -> Result<()> {
        let schema_str = fs::read_to_string(PathBuf::from("autopilot.schema.json"))
            .context("Failed to read schema file")?;
        let schema_json: Value =
            serde_json::from_str(&schema_str).context("Failed to parse JSON schema")?;

        let json_value =
            serde_json::to_value(config).context("Failed to convert YAML to JSON 2")?;
        if let Err(err) = jsonschema::validate(&schema_json, &json_value) {
            anyhow::bail!("Schema validation failed: {}", err);
        }

        Ok(())
    }

    pub fn load_config(yaml_path: &Path) -> Result<Self> {
        let yaml_config = fs::read_to_string(PathBuf::from(yaml_path))
            .context("Should have been able to read the file")?;

        let config: Config = serde_yaml2::from_str(yaml_config.as_str())
            .context("Failed to convert YAML to JSON")?;
        Self::validate_config(&config)?;

        Ok(config)
    }
}
