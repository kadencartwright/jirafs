use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::MutexGuard;
use std::time::{Duration, UNIX_EPOCH};

use fuser::{
    Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo,
    OpenAccMode, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    ReplyWrite, Request, TimeOrNow,
};

use crate::cache::InMemoryCache;
use crate::jira::JiraClient;
use crate::logging;
use crate::sync_state::SyncState;
use crate::warmup::sync_issues;

const TTL: Duration = Duration::from_secs(1);

const INO_SYNC_META: INodeNo = INodeNo(0x1000);
const INO_LAST_SYNC: INodeNo = INodeNo(0x1001);
const INO_LAST_FULL_SYNC: INodeNo = INodeNo(0x1005);
const INO_SECONDS_TO_NEXT: INodeNo = INodeNo(0x1002);
const INO_MANUAL_REFRESH: INodeNo = INodeNo(0x1003);
const INO_FULL_REFRESH: INodeNo = INodeNo(0x1004);
const INO_WORKSPACES: INodeNo = INodeNo(0x2000);

#[derive(Debug, Clone, Copy)]
enum IssueFileKind {
    Main,
    CommentsMarkdown,
}

#[derive(Debug, Clone)]
enum Node {
    Root,
    SyncMeta,
    Workspaces,
    Workspace { name: String },
    Issue { key: String, kind: IssueFileKind },
    SyncMetaFile,
}

#[derive(Debug, Default)]
struct FsState {
    nodes: HashMap<INodeNo, Node>,
}

#[derive(Debug)]
pub struct JiraFuseFs {
    uid: u32,
    gid: u32,
    workspaces: Vec<(String, String)>,
    jira: Arc<JiraClient>,
    cache: Arc<InMemoryCache>,
    sync_budget: usize,
    sync_state: Arc<SyncState>,
    initial_sync_started: AtomicBool,
    state: std::sync::Mutex<FsState>,
}

impl JiraFuseFs {
    pub fn new(
        uid: u32,
        gid: u32,
        workspaces: Vec<(String, String)>,
        jira: Arc<JiraClient>,
        cache: Arc<InMemoryCache>,
        sync_budget: usize,
        sync_state: Arc<SyncState>,
    ) -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(INodeNo::ROOT, Node::Root);

        Self {
            uid,
            gid,
            workspaces,
            jira,
            cache,
            sync_budget,
            sync_state,
            initial_sync_started: AtomicBool::new(false),
            state: std::sync::Mutex::new(FsState { nodes }),
        }
    }

    fn spawn_initial_sync(&self) {
        if self.initial_sync_started.swap(true, Ordering::Relaxed) {
            return;
        }

        let jira = Arc::clone(&self.jira);
        let cache = Arc::clone(&self.cache);
        let workspaces = self.workspaces.clone();
        let sync_budget = self.sync_budget;
        let sync_state = Arc::clone(&self.sync_state);

        std::thread::spawn(move || {
            if !sync_state.mark_sync_start() {
                return;
            }

            logging::info("starting initial sync after mount...");
            let sync_result = sync_issues(&jira, &cache, &workspaces, sync_budget, false);

            sync_state.mark_sync_complete();
            sync_state.mark_sync_end();

            logging::info(format!(
                "initial sync complete: cached={} skipped={} errors={}",
                sync_result.issues_cached,
                sync_result.issues_skipped,
                sync_result.errors.len()
            ));

            if !sync_result.errors.is_empty() {
                for err in &sync_result.errors {
                    logging::warn(format!("sync error: {}", err));
                }
            }
        });
    }

    fn workspace_names(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn dir_attr(&self, ino: INodeNo) -> FileAttr {
        FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::Directory,
            perm: 0o555,
            nlink: 2,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
    }

    fn file_attr(&self, ino: INodeNo, size: u64, writable: bool) -> FileAttr {
        FileAttr {
            ino,
            size,
            blocks: 1,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::RegularFile,
            perm: if writable { 0o644 } else { 0o444 },
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
    }

    fn workspace_for_inode(&self, ino: INodeNo) -> Option<String> {
        let guard = self.state_guard();
        if let Some(Node::Workspace { name }) = guard.nodes.get(&ino) {
            return Some(name.clone());
        }

        self.workspace_names()
            .into_iter()
            .find(|workspace| inode_for_workspace(workspace) == ino)
    }

    fn node_for_inode(&self, ino: INodeNo) -> Option<Node> {
        if ino == INO_SYNC_META {
            return Some(Node::SyncMeta);
        }
        if ino == INO_WORKSPACES {
            return Some(Node::Workspaces);
        }
        if ino == INO_LAST_SYNC
            || ino == INO_LAST_FULL_SYNC
            || ino == INO_SECONDS_TO_NEXT
            || ino == INO_MANUAL_REFRESH
            || ino == INO_FULL_REFRESH
        {
            return Some(Node::SyncMetaFile);
        }

        self.state_guard().nodes.get(&ino).cloned()
    }

    fn upsert_node(&self, ino: INodeNo, node: Node) {
        self.state_guard().nodes.insert(ino, node);
    }

    fn state_guard(&self) -> MutexGuard<'_, FsState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                logging::warn("recovering poisoned mutex: fs state");
                poisoned.into_inner()
            }
        }
    }

    fn issue_exists_in_workspace(&self, workspace: &str, issue_key: &str) -> Result<bool, Errno> {
        let issues = self.workspace_issues(workspace)?;
        Ok(issues.iter().any(|i| i.key == issue_key))
    }

    fn workspace_issues(&self, workspace: &str) -> Result<Vec<crate::jira::IssueRef>, Errno> {
        if let Some(snapshot) = self.cache.get_workspace_issues_snapshot(workspace) {
            return Ok(snapshot.issues);
        }

        Ok(Vec::new())
    }

    fn issue_bytes(&self, issue_key: &str) -> Result<Vec<u8>, Errno> {
        self.cache
            .get_issue_markdown_stale_safe(issue_key, || Err(Errno::EAGAIN))
            .or_else(|_| {
                Ok(format!(
                    "# {}\n\nNot yet available in local cache. Wait for sync interval or trigger manual refresh via `.sync_meta/manual_refresh`.\n",
                    issue_key
                )
                .into_bytes())
            })
    }

    fn issue_main_size(&self, issue_key: &str) -> u64 {
        self.cache
            .cached_issue_len(issue_key)
            .or_else(|| self.cache.persistent_issue_len(issue_key))
            .unwrap_or(0)
    }

    fn issue_comments_markdown_bytes(&self, issue_key: &str) -> Vec<u8> {
        if let Some(bytes) = self.cache.persistent_comments_md(issue_key) {
            return bytes;
        }
        format!(
            "# {} comments\n\nComments sidecar is only populated during sync.\n",
            issue_key
        )
        .into_bytes()
    }

    fn issue_sidecar_size(&self, issue_key: &str, kind: IssueFileKind) -> u64 {
        match kind {
            IssueFileKind::Main => self.issue_main_size(issue_key),
            IssueFileKind::CommentsMarkdown => self
                .cache
                .persistent_comments_md_len(issue_key)
                .unwrap_or(64),
        }
    }

    fn sync_meta_file_content(&self, ino: INodeNo) -> Vec<u8> {
        if ino == INO_LAST_SYNC {
            if let Some(last) = self.sync_state.last_sync() {
                let secs = last.elapsed().as_secs();
                return format!("{} seconds ago\n", secs).into_bytes();
            } else {
                return b"never\n".to_vec();
            }
        }
        if ino == INO_LAST_FULL_SYNC {
            if let Some(last) = self.sync_state.last_full_sync() {
                let secs = last.elapsed().as_secs();
                return format!("{} seconds ago\n", secs).into_bytes();
            } else {
                return b"never\n".to_vec();
            }
        }
        if ino == INO_SECONDS_TO_NEXT {
            let secs = self.sync_state.seconds_until_next_sync();
            return format!("{}\n", secs).into_bytes();
        }
        if ino == INO_MANUAL_REFRESH {
            if self.sync_state.is_sync_in_progress() {
                return b"sync in progress\n".to_vec();
            } else {
                return b"write '1' or 'true' to trigger sync\n".to_vec();
            }
        }
        if ino == INO_FULL_REFRESH {
            if self.sync_state.is_sync_in_progress() {
                return b"sync in progress\n".to_vec();
            } else {
                return b"write '1' or 'true' to trigger full upsert sync\n".to_vec();
            }
        }
        b"unknown\n".to_vec()
    }
}

impl Filesystem for JiraFuseFs {
    fn init(&mut self, _req: &Request, _config: &mut fuser::KernelConfig) -> io::Result<()> {
        self.spawn_initial_sync();
        Ok(())
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        if parent == INodeNo::ROOT {
            if name == OsStr::new(".sync_meta") {
                reply.entry(&TTL, &self.dir_attr(INO_SYNC_META), Generation(0));
                return;
            }
            if name == OsStr::new("workspaces") {
                reply.entry(&TTL, &self.dir_attr(INO_WORKSPACES), Generation(0));
                return;
            }
            reply.error(Errno::ENOENT);
            return;
        }

        if parent == INO_SYNC_META {
            if name == OsStr::new("last_sync") {
                let content = self.sync_meta_file_content(INO_LAST_SYNC);
                reply.entry(
                    &TTL,
                    &self.file_attr(INO_LAST_SYNC, content.len() as u64, false),
                    Generation(0),
                );
                return;
            }
            if name == OsStr::new("last_full_sync") {
                let content = self.sync_meta_file_content(INO_LAST_FULL_SYNC);
                reply.entry(
                    &TTL,
                    &self.file_attr(INO_LAST_FULL_SYNC, content.len() as u64, false),
                    Generation(0),
                );
                return;
            }
            if name == OsStr::new("seconds_to_next_sync") {
                let content = self.sync_meta_file_content(INO_SECONDS_TO_NEXT);
                reply.entry(
                    &TTL,
                    &self.file_attr(INO_SECONDS_TO_NEXT, content.len() as u64, false),
                    Generation(0),
                );
                return;
            }
            if name == OsStr::new("manual_refresh") {
                let content = self.sync_meta_file_content(INO_MANUAL_REFRESH);
                reply.entry(
                    &TTL,
                    &self.file_attr(INO_MANUAL_REFRESH, content.len() as u64, true),
                    Generation(0),
                );
                return;
            }
            if name == OsStr::new("full_refresh") {
                let content = self.sync_meta_file_content(INO_FULL_REFRESH);
                reply.entry(
                    &TTL,
                    &self.file_attr(INO_FULL_REFRESH, content.len() as u64, true),
                    Generation(0),
                );
                return;
            }
            reply.error(Errno::ENOENT);
            return;
        }

        if parent == INO_WORKSPACES {
            if let Some(workspace) = self
                .workspace_names()
                .iter()
                .find(|w| name == OsStr::new(w))
            {
                let ino = inode_for_workspace(workspace);
                self.upsert_node(
                    ino,
                    Node::Workspace {
                        name: workspace.to_string(),
                    },
                );
                reply.entry(&TTL, &self.dir_attr(ino), Generation(0));
                return;
            }
            reply.error(Errno::ENOENT);
            return;
        }

        let Some(workspace) = self.workspace_for_inode(parent) else {
            reply.error(Errno::ENOENT);
            return;
        };

        let Some(file_name) = name.to_str() else {
            reply.error(Errno::ENOENT);
            return;
        };

        let (issue_key, kind) = if let Some(value) = file_name.strip_suffix(".comments.md") {
            (value, IssueFileKind::CommentsMarkdown)
        } else if let Some(value) = file_name.strip_suffix(".md") {
            (value, IssueFileKind::Main)
        } else {
            reply.error(Errno::ENOENT);
            return;
        };

        match self.issue_exists_in_workspace(&workspace, issue_key) {
            Ok(true) => {
                let ino = inode_for_issue_kind(&workspace, issue_key, kind);
                self.upsert_node(
                    ino,
                    Node::Issue {
                        key: issue_key.to_string(),
                        kind,
                    },
                );
                let size = self.issue_sidecar_size(issue_key, kind);
                reply.entry(&TTL, &self.file_attr(ino, size, false), Generation(0));
            }
            Ok(false) => reply.error(Errno::ENOENT),
            Err(err) => reply.error(err),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        if ino == INodeNo::ROOT {
            reply.attr(&TTL, &self.dir_attr(ino));
            return;
        }

        if ino == INO_SYNC_META || ino == INO_WORKSPACES {
            reply.attr(&TTL, &self.dir_attr(ino));
            return;
        }

        if ino == INO_LAST_SYNC
            || ino == INO_LAST_FULL_SYNC
            || ino == INO_SECONDS_TO_NEXT
            || ino == INO_MANUAL_REFRESH
            || ino == INO_FULL_REFRESH
        {
            let content = self.sync_meta_file_content(ino);
            let writable = ino == INO_MANUAL_REFRESH || ino == INO_FULL_REFRESH;
            reply.attr(&TTL, &self.file_attr(ino, content.len() as u64, writable));
            return;
        }

        if let Some(workspace) = self
            .workspace_names()
            .iter()
            .find(|w| inode_for_workspace(w) == ino)
        {
            self.upsert_node(
                ino,
                Node::Workspace {
                    name: workspace.clone(),
                },
            );
            reply.attr(&TTL, &self.dir_attr(ino));
            return;
        }

        match self.node_for_inode(ino) {
            Some(Node::Issue { key, kind }) => {
                let size = self.issue_sidecar_size(&key, kind);
                reply.attr(&TTL, &self.file_attr(ino, size, false));
            }
            Some(Node::Workspace { .. }) => reply.attr(&TTL, &self.dir_attr(ino)),
            _ => reply.error(Errno::ENOENT),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        if ino == INodeNo::ROOT {
            let entries: Vec<(INodeNo, FileType, String)> = vec![
                (INodeNo::ROOT, FileType::Directory, ".".to_string()),
                (INodeNo::ROOT, FileType::Directory, "..".to_string()),
                (INO_SYNC_META, FileType::Directory, ".sync_meta".to_string()),
                (
                    INO_WORKSPACES,
                    FileType::Directory,
                    "workspaces".to_string(),
                ),
            ];

            for (idx, (entry_ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
                if reply.add(*entry_ino, (idx + 1) as u64, *kind, name) {
                    break;
                }
            }
            reply.ok();
            return;
        }

        if ino == INO_SYNC_META {
            let entries: Vec<(INodeNo, FileType, String)> = vec![
                (INO_SYNC_META, FileType::Directory, ".".to_string()),
                (INodeNo::ROOT, FileType::Directory, "..".to_string()),
                (
                    INO_LAST_SYNC,
                    FileType::RegularFile,
                    "last_sync".to_string(),
                ),
                (
                    INO_LAST_FULL_SYNC,
                    FileType::RegularFile,
                    "last_full_sync".to_string(),
                ),
                (
                    INO_SECONDS_TO_NEXT,
                    FileType::RegularFile,
                    "seconds_to_next_sync".to_string(),
                ),
                (
                    INO_MANUAL_REFRESH,
                    FileType::RegularFile,
                    "manual_refresh".to_string(),
                ),
                (
                    INO_FULL_REFRESH,
                    FileType::RegularFile,
                    "full_refresh".to_string(),
                ),
            ];

            for (idx, (entry_ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
                if reply.add(*entry_ino, (idx + 1) as u64, *kind, name) {
                    break;
                }
            }
            reply.ok();
            return;
        }

        if ino == INO_WORKSPACES {
            let mut entries: Vec<(INodeNo, FileType, String)> = vec![
                (INO_WORKSPACES, FileType::Directory, ".".to_string()),
                (INodeNo::ROOT, FileType::Directory, "..".to_string()),
            ];

            for workspace in self.workspace_names() {
                let w_ino = inode_for_workspace(&workspace);
                self.upsert_node(
                    w_ino,
                    Node::Workspace {
                        name: workspace.clone(),
                    },
                );
                entries.push((w_ino, FileType::Directory, workspace));
            }

            for (idx, (entry_ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
                if reply.add(*entry_ino, (idx + 1) as u64, *kind, name) {
                    break;
                }
            }
            reply.ok();
            return;
        }

        let Some(workspace) = self.workspace_for_inode(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };

        let issues = match self.workspace_issues(&workspace) {
            Ok(items) => items,
            Err(err) => {
                reply.error(err);
                return;
            }
        };

        let mut entries: Vec<(INodeNo, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (INO_WORKSPACES, FileType::Directory, "..".to_string()),
        ];

        for issue in issues {
            let issue_ino = inode_for_issue_kind(&workspace, &issue.key, IssueFileKind::Main);
            let comments_md_ino =
                inode_for_issue_kind(&workspace, &issue.key, IssueFileKind::CommentsMarkdown);
            self.upsert_node(
                issue_ino,
                Node::Issue {
                    key: issue.key.clone(),
                    kind: IssueFileKind::Main,
                },
            );
            self.upsert_node(
                comments_md_ino,
                Node::Issue {
                    key: issue.key.clone(),
                    kind: IssueFileKind::CommentsMarkdown,
                },
            );
            entries.push((
                issue_ino,
                FileType::RegularFile,
                format!("{}.md", issue.key),
            ));
            entries.push((
                comments_md_ino,
                FileType::RegularFile,
                format!("{}.comments.md", issue.key),
            ));
        }

        for (idx, (entry_ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
            if reply.add(*entry_ino, (idx + 1) as u64, *kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let is_writable_file = ino == INO_MANUAL_REFRESH || ino == INO_FULL_REFRESH;

        if flags.acc_mode() != OpenAccMode::O_RDONLY && !is_writable_file {
            reply.error(Errno::EROFS);
            return;
        }

        match self.node_for_inode(ino) {
            Some(Node::Issue { .. }) | Some(Node::SyncMetaFile) => {
                reply.opened(FileHandle(0), FopenFlags::empty())
            }
            Some(Node::Workspace { .. })
            | Some(Node::SyncMeta)
            | Some(Node::Workspaces)
            | Some(Node::Root) => reply.error(Errno::EISDIR),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: ReplyData,
    ) {
        if ino == INO_LAST_SYNC
            || ino == INO_LAST_FULL_SYNC
            || ino == INO_SECONDS_TO_NEXT
            || ino == INO_MANUAL_REFRESH
            || ino == INO_FULL_REFRESH
        {
            let data = self.sync_meta_file_content(ino);
            let start = offset as usize;
            if start >= data.len() {
                reply.data(&[]);
                return;
            }
            let end = start.saturating_add(size as usize).min(data.len());
            reply.data(&data[start..end]);
            return;
        }

        let Some(Node::Issue { key, kind }) = self.node_for_inode(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };

        let data = match kind {
            IssueFileKind::Main => self.issue_bytes(&key),
            IssueFileKind::CommentsMarkdown => Ok(self.issue_comments_markdown_bytes(&key)),
        };

        let data = match data {
            Ok(bytes) => bytes,
            Err(err) => {
                reply.error(err);
                return;
            }
        };

        let start = offset as usize;
        if start >= data.len() {
            reply.data(&[]);
            return;
        }
        let end = start.saturating_add(size as usize).min(data.len());
        reply.data(&data[start..end]);
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: fuser::WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: ReplyWrite,
    ) {
        if ino != INO_MANUAL_REFRESH && ino != INO_FULL_REFRESH {
            reply.error(Errno::EROFS);
            return;
        }

        if offset != 0 {
            reply.error(Errno::EINVAL);
            return;
        }

        let content = String::from_utf8_lossy(data).to_lowercase();
        let trimmed = content.trim();

        if trimmed == "1" || trimmed == "true" {
            if ino == INO_FULL_REFRESH {
                self.sync_state.trigger_manual_full();
                logging::info("manual full sync triggered via .sync_meta/full_refresh");
            } else {
                self.sync_state.trigger_manual();
                logging::info("manual sync triggered via .sync_meta/manual_refresh");
            }
        }

        reply.written(data.len() as u32);
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        if ino == INO_MANUAL_REFRESH || ino == INO_FULL_REFRESH {
            let content = self.sync_meta_file_content(ino);
            reply.attr(&TTL, &self.file_attr(ino, content.len() as u64, true));
            return;
        }
        reply.error(Errno::EROFS);
    }
}

pub fn inode_for_workspace(workspace: &str) -> INodeNo {
    INodeNo(namespace_hash(0x11, workspace.as_bytes()))
}

pub fn inode_for_issue(workspace: &str, issue_key: &str) -> INodeNo {
    let mut bytes = workspace.as_bytes().to_vec();
    bytes.push(b'/');
    bytes.extend_from_slice(issue_key.as_bytes());
    INodeNo(namespace_hash(0x22, &bytes))
}

fn inode_for_issue_kind(workspace: &str, issue_key: &str, kind: IssueFileKind) -> INodeNo {
    match kind {
        IssueFileKind::Main => inode_for_issue(workspace, issue_key),
        IssueFileKind::CommentsMarkdown => {
            let mut bytes = workspace.as_bytes().to_vec();
            bytes.push(b'/');
            bytes.extend_from_slice(issue_key.as_bytes());
            bytes.extend_from_slice(b"#comments.md");
            INodeNo(namespace_hash(0x23, &bytes))
        }
    }
}

fn namespace_hash(namespace: u8, bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    hash ^= u64::from(namespace);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    let value = hash | (1_u64 << 63);
    if value == 1 {
        3
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_inode_is_deterministic() {
        assert_eq!(
            inode_for_workspace("default"),
            inode_for_workspace("default")
        );
    }

    #[test]
    fn distinct_workspace_inodes() {
        assert_ne!(inode_for_workspace("alpha"), inode_for_workspace("beta"));
    }

    #[test]
    fn issue_inode_is_deterministic_and_namespaced() {
        let a = inode_for_issue("default", "PROJ-1");
        let b = inode_for_issue("default", "PROJ-1");
        let c = inode_for_issue("default", "PROJ-2");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, inode_for_workspace("default"));
    }
}
