# Jirafs (pronounced Giraffes)

Read-only Rust FUSE filesystem that exposes Jira issues as markdown files.

<img src="apps/desktop/src-tauri/icons/icon.png" width="500" alt="Alt Text">

## Prerequisites

- Rust toolchain (`rustup`, `cargo`)
- `just` task runner (optional, recommended)

Pinned toolchain versions used in CI:

- Node.js `20.12.2`
- Rust `1.84.0`

### Linux

- FUSE support enabled
- FUSE userspace library headers (for example, `libfuse3-dev` on Debian/Ubuntu)

### macOS

- macFUSE and pkg-config (for example, `brew install macfuse pkgconf`)
- On Apple Silicon, allow third-party kernel extensions for macFUSE

## Build

```bash
just build
```

Raw Cargo alternative:

```bash
cargo build --locked
```

## Install

Install the CLI, build/install the desktop UI launcher, and bootstrap a default config in the runtime lookup path:

```bash
just install
```

`just install` now also:

- builds the desktop UI from `apps/desktop`
- installs desktop binary at `~/.local/bin/jirafs-desktop`
- Linux: creates desktop entry at `${XDG_DATA_HOME:-~/.local/share}/applications/jirafs-desktop.desktop`
- macOS: creates app bundle at `~/Applications/jirafs Desktop.app`

Config bootstrap remains non-destructive. If `config.toml` already exists at the resolved destination, install keeps going and skips writing a new config file.

Raw Cargo alternative:

```bash
cargo install --path . --locked
```

## Quality Checks

Run these before opening a PR:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
```

Optional API docs validation:

```bash
cargo doc --no-deps
```

CI runs on Linux and macOS to validate compilation and quality gates. Runtime mount and unmount behavior still needs a real host-level smoke test.

## Configure Jira

Create a TOML config file at one of these paths:

- `$XDG_CONFIG_HOME/jirafs/config.toml` (preferred when `XDG_CONFIG_HOME` is set)
- `~/.config/jirafs/config.toml` (fallback when `XDG_CONFIG_HOME` is unset)

Start from the checked-in example:

```bash
just install
```

Manual alternative:

```bash
mkdir -p ~/.config/jirafs
cp config.example.toml ~/.config/jirafs/config.toml
```

Then edit `~/.config/jirafs/config.toml` with your Jira values:

```bash
cat ~/.config/jirafs/config.toml
```

Authentication uses Jira Cloud basic auth with email + API token.

Runtime config is a hard cutover to TOML; environment variables and `.env` are no longer read at startup.

You can override config location and individual values with CLI flags. CLI values take precedence over TOML values.
Use `-c` as a short alias for `--config`, and `--help` (or `-h`) to print CLI usage.

```bash
cargo run -- \
  --config /path/to/config.toml \
  --jira-base-url https://your-domain.atlassian.net \
  --jira-email you@example.com \
  --jira-api-token ... \
  --jira-workspace "default=project in (PROJ, OPS) ORDER BY updated DESC" \
  --jira-workspace "ops=project = OPS ORDER BY updated DESC" \
  --cache-db-path /tmp/jirafs-cache.db \
  --cache-ttl-secs 30 \
  --sync-budget 1000 \
  --sync-interval-secs 60 \
  --metrics-interval-secs 60 \
  --logging-debug false \
  /tmp/jirafs-mnt
```

Migration key mapping:

- `JIRA_BASE_URL` -> `jira.base_url`
- `JIRA_EMAIL` -> `jira.email`
- `JIRA_API_TOKEN` -> `jira.api_token`
- `JIRA_WORKSPACES` -> `jira.workspaces.<name>.jql`
- `JIRA_CACHE_TTL_SECS` -> `cache.ttl_secs`
- `JIRAFS_CACHE_DB` -> `cache.db_path`
- `JIRAFS_SYNC_BUDGET` -> `sync.budget`
- `JIRAFS_SYNC_INTERVAL_SECS` -> `sync.interval_secs`
- `JIRAFS_METRICS_INTERVAL_SECS` -> `metrics.interval_secs`
- `JIRAFS_DEBUG` -> `logging.debug`

## Mount

Create a mountpoint and run:

```bash
just run /tmp/jirafs-mnt
```

To run with an explicit config file path:

```bash
just run-with-config /path/to/config.toml /tmp/jirafs-mnt
```

Raw Cargo alternative:

```bash
mkdir -p /tmp/jirafs-mnt
cargo run --locked -- /tmp/jirafs-mnt
```

In another terminal:

```bash
ls -la /tmp/jirafs-mnt
ls -la /tmp/jirafs-mnt/workspaces
ls -la /tmp/jirafs-mnt/workspaces/default
cat /tmp/jirafs-mnt/workspaces/default/PROJ-123.md
grep -R "in_progress" /tmp/jirafs-mnt/workspaces
```

The filesystem is effectively read-only for issue content. The only supported writes are sync triggers to `.sync_meta/manual_refresh` and `.sync_meta/full_refresh`.

Notes:
- `cache.db_path` enables persistent issue markdown cache (SQLite).
- Workspace listings are hydrated from persistence on startup.
- Sync warmup prefetches recent issues up to `sync.budget`.
- Periodic cache/API counters are emitted to stderr.
- Workspace directory listings serve cached results immediately.
- `logging.debug = true` enables verbose debug logs for refresh/retry/cache flow.

## Auto-start Services

`jirafs` can auto-mount at login with a single per-user service instance:

- Linux: `systemd --user` unit `jirafs.service`
- macOS: launchd LaunchAgent `com.jirafs.mount`

Default service mountpoint is `~/jirafs`.

Prerequisites:

1. Binary is installed and on `PATH`: `just install`
2. Config exists and is valid at one of:
   - `$XDG_CONFIG_HOME/jirafs/config.toml`
   - `~/.config/jirafs/config.toml`

Install service files:

```bash
just service-install
```

Optional explicit paths:

```bash
just service-install ~/jirafs /path/to/config.toml
```

Enable/start at login:

```bash
just service-enable
```

Check status:

```bash
just service-status
```

View logs:

```bash
just service-logs
```

Stop without uninstall:

```bash
just service-stop
```

Disable autostart:

```bash
just service-disable
```

Remove managed service files:

```bash
just service-uninstall
```

Troubleshooting:

- `config file not found`: create config with `just install` or pass explicit path to `just service-install <mountpoint> <config_path>`.
- `jirafs binary not found`: run `just install` and ensure your shell `PATH` includes Cargo install location.
- stale mountpoint: unmount manually, then restart service.
- prefer stable mount paths (like `~/jirafs`) for services; avoid `/tmp` for login-persistent mounts.

## Unmount

Linux:

```bash
fusermount3 -u /tmp/jirafs-mnt
```

If your distro provides `fusermount` instead of `fusermount3`, use that command.

macOS:

```bash
umount /tmp/jirafs-mnt
```

## Desktop App (Tauri)

The repository includes an additive desktop control UI at `apps/desktop` for Linux tray / macOS menubar status and sync actions.

Desktop prerequisites (required by `just install`):

- Node.js `20.12.2`
- Rust `1.84.0`
- Linux: `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`

Install and run:

```bash
npm --prefix apps/desktop ci
npm --prefix apps/desktop run tauri:dev
```

After `just install`, launch the desktop UI with:

- Linux: app launcher entry `jirafs Desktop` (or `~/.local/bin/jirafs-desktop`)
- macOS: `~/Applications/jirafs Desktop.app`

Desktop checks:

```bash
npm --prefix apps/desktop run biome:check
npm --prefix apps/desktop run build
cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml
```
