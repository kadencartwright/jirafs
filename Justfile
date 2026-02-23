set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

build:
    cargo build --locked

run mountpoint="/tmp/fs-jira-mnt":
    mkdir -p "{{mountpoint}}"
    cargo run --locked -- "{{mountpoint}}"

run-with-config config_path mountpoint="/tmp/fs-jira-mnt":
    mkdir -p "{{mountpoint}}"
    cargo run --locked -- --config "{{config_path}}" "{{mountpoint}}"

install:
    cargo install --path . --locked
    if [ -n "${XDG_CONFIG_HOME:-}" ]; then \
      config_dir="${XDG_CONFIG_HOME}/fs-jira"; \
    elif [ -n "${HOME:-}" ]; then \
      config_dir="${HOME}/.config/fs-jira"; \
    else \
      echo "failed to resolve config path: HOME is not set and XDG_CONFIG_HOME is unset" >&2; \
      exit 1; \
    fi; \
    mkdir -p "$config_dir"; \
    config_path="$config_dir/config.toml"; \
    if [ -e "$config_path" ]; then \
      echo "refusing to overwrite existing config: $config_path" >&2; \
      exit 1; \
    fi; \
    cp config.example.toml "$config_path"; \
    echo "created default config: $config_path"
