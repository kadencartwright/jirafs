# fs-jira

Read-only Rust FUSE filesystem that exposes Jira issues as markdown files.

## Prerequisites

- Rust toolchain (`rustup`, `cargo`)

### Linux

- FUSE support enabled
- FUSE userspace library headers (for example, `libfuse3-dev` on Debian/Ubuntu)

### macOS

- macFUSE and pkg-config (for example, `brew install macfuse pkgconf`)
- On Apple Silicon, allow third-party kernel extensions for macFUSE

## Build

```bash
cargo build
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

- `$XDG_CONFIG_HOME/fs-jira/config.toml` (preferred when `XDG_CONFIG_HOME` is set)
- `~/.config/fs-jira/config.toml` (fallback when `XDG_CONFIG_HOME` is unset)

Start from the checked-in example:

```bash
mkdir -p ~/.config/fs-jira
cp config.example.toml ~/.config/fs-jira/config.toml
```

Then edit `~/.config/fs-jira/config.toml` with your Jira values:

```bash
cat ~/.config/fs-jira/config.toml
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
  --jira-project PROJ \
  --jira-project OPS \
  --cache-db-path /tmp/fs-jira-cache.db \
  --cache-ttl-secs 30 \
  --sync-budget 1000 \
  --sync-interval-secs 60 \
  --metrics-interval-secs 60 \
  --logging-debug false \
  /tmp/fs-jira-mnt
```

Migration key mapping:

- `JIRA_BASE_URL` -> `jira.base_url`
- `JIRA_EMAIL` -> `jira.email`
- `JIRA_API_TOKEN` -> `jira.api_token`
- `JIRA_PROJECTS` -> `jira.projects`
- `JIRA_CACHE_TTL_SECS` -> `cache.ttl_secs`
- `FS_JIRA_CACHE_DB` -> `cache.db_path`
- `FS_JIRA_SYNC_BUDGET` -> `sync.budget`
- `FS_JIRA_SYNC_INTERVAL_SECS` -> `sync.interval_secs`
- `FS_JIRA_METRICS_INTERVAL_SECS` -> `metrics.interval_secs`
- `FS_JIRA_DEBUG` -> `logging.debug`

## Mount

Create a mountpoint and run:

```bash
mkdir -p /tmp/fs-jira-mnt
cargo run -- /tmp/fs-jira-mnt
```

In another terminal:

```bash
ls -la /tmp/fs-jira-mnt
ls -la /tmp/fs-jira-mnt/PROJ
cat /tmp/fs-jira-mnt/PROJ/PROJ-123.md
grep -R "Status:" /tmp/fs-jira-mnt
```

The filesystem is mounted read-only. Writes should fail.

Notes:
- `cache.db_path` enables persistent issue markdown cache (SQLite).
- Project listings are seeded at startup (best effort) before mount.
- Sync warmup prefetches recent issues up to `sync.budget`.
- Periodic cache/API counters are emitted to stderr.
- Project directory listings serve cached results immediately and refresh stale listings in the background.
- `logging.debug = true` enables verbose debug logs for refresh/retry/cache flow.

## Unmount

Linux:

```bash
fusermount3 -u /tmp/fs-jira-mnt
```

If your distro provides `fusermount` instead of `fusermount3`, use that command.

macOS:

```bash
umount /tmp/fs-jira-mnt
```
