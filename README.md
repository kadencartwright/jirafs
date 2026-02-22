# fs-jira

Read-only Rust FUSE filesystem that exposes Jira issues as markdown files.

## Prerequisites

- Linux with FUSE support
- Rust toolchain (`rustup`, `cargo`)
- FUSE userspace library headers (for example, `libfuse3-dev` on Debian/Ubuntu)

## Build

```bash
cargo build
```

## Configure Jira

Set required environment variables:

```bash
export JIRA_BASE_URL="https://your-domain.atlassian.net"
export JIRA_EMAIL="you@example.com"
export JIRA_API_TOKEN="..."
export JIRA_PROJECTS="PROJ,OPS"

# Optional tuning
export JIRA_CACHE_TTL_SECS="30"
export FS_JIRA_CACHE_DB="/tmp/fs-jira-cache.db"
export FS_JIRA_WARMUP_BUDGET="25"
export FS_JIRA_METRICS_INTERVAL_SECS="60"
export FS_JIRA_DEBUG="1"
```

Authentication uses Jira Cloud basic auth with email + API token.

You can also place these values in a local `.env` file for testing; it is auto-loaded at startup.

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
- `FS_JIRA_CACHE_DB` enables persistent issue markdown cache (SQLite).
- Project listings are seeded at startup (best effort) before mount.
- Warmup prefetches recent issues up to `FS_JIRA_WARMUP_BUDGET`.
- Periodic cache/API counters are emitted to stderr.
- Project directory listings serve cached results immediately and refresh stale listings in the background.
- `FS_JIRA_DEBUG=1` enables verbose debug logs for refresh/retry/cache flow.

## Unmount

```bash
fusermount3 -u /tmp/fs-jira-mnt
```

If your distro provides `fusermount` instead of `fusermount3`, use that command.
