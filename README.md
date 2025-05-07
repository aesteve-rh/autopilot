<!--
SPDX-FileCopyrightText: 2025 Albert Esteve <aesteve@redhat.com>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# AutoPilot

Automate workflows with ease - messages, commands, loops, remote execution,
and styled terminal output.

# Features
- Sequential tasks execution with back and forth support
- Styled messages with colors, styles, and configurable display speed
- Commands:
  - Local or remote (via `ssh`) execution
  - Run as regular or privileged user
  - Support list of commands
- Support environment variables
  - Remote connection host
  - Remote connection user
  - Remote connection password
  - Remote sudo user
  - Remote sudo password
- Loops with configurable delay
- Hide commands' output (`stdout` and/or `stderr`) for silent execution
- YAML-based, human-friendly configuration
- Beautiful display in a terminal user interface (TUI)

# Installation

## Prerequisites

### Install Rust

```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Install packages
```shell
sudo dnf install -y gcc openssl-devel
```

## Clone the project
```shell
git clone https://github.com/aesteve-rh/autopilot.git
cd autopilot
```

## Build

```shell
cargo build --release
```

# Usage

## Create workflow

Create a YAML file like this:

```yaml
stages:
  - name: "Deploy App"
    actions:
      - type: message
        text: "Starting Deployment..."
        style:
          color: cyan
          bold: true
        speed: 50

      - type: command
        command: "echo 'Deploying services...'"
        style:
          color: "yellow"
          italic: true

      # Multiple commands support.
      - type: command
        command:
          - restart-service.sh
          - echo 'Service restarted!'
        hide_stdout: true
        hide_stderr: false
        remote:
          user: admin
          host: server.com
          port: 22
          password: admin1234
        sudo:
          user: root
          password: root1234

      # Environment variables support (security & usability).
      - type: command
        command: config_collector.sh 1.1.1.1
        remote:
          user: $env:REMOTE_USER
          host: $env:REMOTE_HOST
          port: 22
          password: $env:REMOTE_PASSWORD
        sudo:
          user: $env:REMOTE_SUDO_USER
          password: $env:REMOTE_SUDO_PASSWORD

      # Local execution in a loop.
      - type: command
        command: mine-bitcoin.sh
        loop:
          times: 3
          delay: 2000
```

**NOTE:** You can see all configuration options documented at [docs/config.md](docs/config.md).

## Run

```console
target/release/autopilot my_tasks.yaml
```

## Output

![](demo.gif)

# Inspiration

This project has been heavily inspired by [nohands](https://github.com/nirs/nohands).

Its purpose and behaviour is similar. But `autopilot` supports more features and
includes a modern user interface to show the demos.

# Contribute

PRs are welcome! Feel free to open issues for new features.
