use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use fuser::{
    Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo,
    OpenAccMode, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, Request,
};

use crate::cache::InMemoryCache;
use crate::jira::JiraClient;
use crate::logging;
use crate::render::render_issue_markdown;

const TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
enum Node {
    Root,
    Project { name: String },
    Issue { key: String },
}

#[derive(Debug, Default)]
struct FsState {
    nodes: HashMap<INodeNo, Node>,
}

#[derive(Debug)]
pub struct JiraFuseFs {
    uid: u32,
    gid: u32,
    projects: Vec<String>,
    jira: Arc<JiraClient>,
    cache: Arc<InMemoryCache>,
    state: Mutex<FsState>,
    project_refresh_in_flight: Arc<Mutex<HashSet<String>>>,
}

impl JiraFuseFs {
    pub fn new(
        uid: u32,
        gid: u32,
        projects: Vec<String>,
        jira: Arc<JiraClient>,
        cache: Arc<InMemoryCache>,
    ) -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(INodeNo::ROOT, Node::Root);

        Self {
            uid,
            gid,
            projects,
            jira,
            cache,
            state: Mutex::new(FsState { nodes }),
            project_refresh_in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
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

    fn file_attr(&self, ino: INodeNo, size: u64) -> FileAttr {
        FileAttr {
            ino,
            size,
            blocks: 1,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::RegularFile,
            perm: 0o444,
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
    }

    fn project_for_inode(&self, ino: INodeNo) -> Option<String> {
        if let Some(Node::Project { name }) = self
            .state
            .lock()
            .expect("state mutex poisoned")
            .nodes
            .get(&ino)
        {
            return Some(name.clone());
        }

        self.projects
            .iter()
            .find(|project| inode_for_project(project) == ino)
            .cloned()
    }

    fn node_for_inode(&self, ino: INodeNo) -> Option<Node> {
        self.state
            .lock()
            .expect("state mutex poisoned")
            .nodes
            .get(&ino)
            .cloned()
    }

    fn upsert_node(&self, ino: INodeNo, node: Node) {
        self.state
            .lock()
            .expect("state mutex poisoned")
            .nodes
            .insert(ino, node);
    }

    fn issue_exists_in_project(&self, project: &str, issue_key: &str) -> Result<bool, Errno> {
        let issues = self.project_issues(project)?;

        Ok(issues.iter().any(|i| i.key == issue_key))
    }

    fn project_issues(&self, project: &str) -> Result<Vec<crate::jira::IssueRef>, Errno> {
        if let Some(snapshot) = self.cache.get_project_issues_snapshot(project) {
            if snapshot.is_stale {
                logging::debug(format!(
                    "project listing stale for {}, scheduling background refresh",
                    project
                ));
                self.spawn_project_refresh(project.to_string());
            }
            return Ok(snapshot.issues);
        }

        logging::debug(format!("project listing cache miss for {}", project));

        self.cache
            .get_project_issues(project, || {
                self.jira
                    .list_project_issue_refs(project)
                    .map_err(|_| Errno::EIO)
            })
            .map_err(|_| Errno::EIO)
    }

    fn spawn_project_refresh(&self, project: String) {
        let mut in_flight = self
            .project_refresh_in_flight
            .lock()
            .expect("refresh set mutex poisoned");
        if in_flight.contains(&project) {
            logging::debug(format!(
                "background refresh already running for {}",
                project
            ));
            return;
        }
        in_flight.insert(project.clone());
        drop(in_flight);

        logging::debug(format!("starting background refresh for {}", project));

        let jira = Arc::clone(&self.jira);
        let cache = Arc::clone(&self.cache);
        let in_flight = Arc::clone(&self.project_refresh_in_flight);

        std::thread::spawn(move || {
            if let Ok(fresh) = jira.list_project_issue_refs(&project) {
                logging::debug(format!(
                    "background refresh succeeded for {} ({} issues)",
                    project,
                    fresh.len()
                ));
                cache.upsert_project_issues(&project, fresh);
            } else {
                logging::warn(format!("background refresh failed for {}", project));
            }

            let mut guard = in_flight.lock().expect("refresh set mutex poisoned");
            guard.remove(&project);
        });
    }

    fn issue_bytes(&self, issue_key: &str) -> Result<Vec<u8>, Errno> {
        self.cache.get_issue_markdown_stale_safe(issue_key, || {
            let issue = self.jira.get_issue(issue_key).map_err(|_| Errno::EIO)?;
            let markdown = render_issue_markdown(&issue).into_bytes();
            Ok((markdown, issue.updated))
        })
    }
}

impl Filesystem for JiraFuseFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        if parent == INodeNo::ROOT {
            if let Some(project) = self.projects.iter().find(|p| name == OsStr::new(p)) {
                let ino = inode_for_project(project);
                self.upsert_node(
                    ino,
                    Node::Project {
                        name: project.to_string(),
                    },
                );
                reply.entry(&TTL, &self.dir_attr(ino), Generation(0));
                return;
            }
            reply.error(Errno::ENOENT);
            return;
        }

        let Some(project) = self.project_for_inode(parent) else {
            reply.error(Errno::ENOENT);
            return;
        };

        let Some(file_name) = name.to_str() else {
            reply.error(Errno::ENOENT);
            return;
        };

        let Some(issue_key) = file_name.strip_suffix(".md") else {
            reply.error(Errno::ENOENT);
            return;
        };

        if !issue_key.starts_with(&(project.clone() + "-")) {
            reply.error(Errno::ENOENT);
            return;
        }

        match self.issue_exists_in_project(&project, issue_key) {
            Ok(true) => {
                let ino = inode_for_issue(&project, issue_key);
                self.upsert_node(
                    ino,
                    Node::Issue {
                        key: issue_key.to_string(),
                    },
                );
                let size = self.cache.cached_issue_len(issue_key).unwrap_or(0);
                reply.entry(&TTL, &self.file_attr(ino, size), Generation(0));
            }
            Ok(false) => reply.error(Errno::ENOENT),
            Err(err) => {
                logging::warn(format!(
                    "lookup failed for project={} issue={} with {:?}",
                    project, issue_key, err
                ));
                reply.error(err)
            }
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        if ino == INodeNo::ROOT {
            reply.attr(&TTL, &self.dir_attr(ino));
            return;
        }

        if let Some(project) = self.projects.iter().find(|p| inode_for_project(p) == ino) {
            self.upsert_node(
                ino,
                Node::Project {
                    name: project.clone(),
                },
            );
            reply.attr(&TTL, &self.dir_attr(ino));
            return;
        }

        match self.node_for_inode(ino) {
            Some(Node::Issue { key, .. }) => {
                let size = self.cache.cached_issue_len(&key).unwrap_or(0);
                reply.attr(&TTL, &self.file_attr(ino, size));
            }
            Some(Node::Project { .. }) => reply.attr(&TTL, &self.dir_attr(ino)),
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
            let mut entries: Vec<(INodeNo, FileType, String)> = vec![
                (INodeNo::ROOT, FileType::Directory, ".".to_string()),
                (INodeNo::ROOT, FileType::Directory, "..".to_string()),
            ];

            for project in &self.projects {
                let p_ino = inode_for_project(project);
                self.upsert_node(
                    p_ino,
                    Node::Project {
                        name: project.clone(),
                    },
                );
                entries.push((p_ino, FileType::Directory, project.clone()));
            }

            for (idx, (entry_ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
                if reply.add(*entry_ino, (idx + 1) as u64, *kind, name) {
                    break;
                }
            }
            reply.ok();
            return;
        }

        let Some(project) = self.project_for_inode(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };

        let issues = match self.project_issues(&project) {
            Ok(items) => items,
            Err(err) => {
                logging::warn(format!(
                    "readdir failed for project={} with {:?}",
                    project, err
                ));
                reply.error(err);
                return;
            }
        };

        let mut entries: Vec<(INodeNo, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (INodeNo::ROOT, FileType::Directory, "..".to_string()),
        ];

        for issue in issues {
            let issue_ino = inode_for_issue(&project, &issue.key);
            self.upsert_node(
                issue_ino,
                Node::Issue {
                    key: issue.key.clone(),
                },
            );
            entries.push((
                issue_ino,
                FileType::RegularFile,
                format!("{}.md", issue.key),
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
        if flags.acc_mode() != OpenAccMode::O_RDONLY {
            reply.error(Errno::EROFS);
            return;
        }

        match self.node_for_inode(ino) {
            Some(Node::Issue { .. }) => reply.opened(FileHandle(0), FopenFlags::empty()),
            Some(Node::Project { .. }) | Some(Node::Root) => reply.error(Errno::EISDIR),
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
        let Some(Node::Issue { key }) = self.node_for_inode(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };

        let data = match self.issue_bytes(&key) {
            Ok(bytes) => bytes,
            Err(err) => {
                logging::warn(format!("read failed for issue={} with {:?}", key, err));
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
}

pub fn inode_for_project(project: &str) -> INodeNo {
    INodeNo(namespace_hash(0x11, project.as_bytes()))
}

pub fn inode_for_issue(project: &str, issue_key: &str) -> INodeNo {
    let mut bytes = project.as_bytes().to_vec();
    bytes.push(b'/');
    bytes.extend_from_slice(issue_key.as_bytes());
    INodeNo(namespace_hash(0x22, &bytes))
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
    fn project_inode_is_deterministic() {
        assert_eq!(inode_for_project("PROJ"), inode_for_project("PROJ"));
    }

    #[test]
    fn distinct_project_inodes() {
        assert_ne!(inode_for_project("AAA"), inode_for_project("BBB"));
    }

    #[test]
    fn issue_inode_is_deterministic_and_namespaced() {
        let a = inode_for_issue("PROJ", "PROJ-1");
        let b = inode_for_issue("PROJ", "PROJ-1");
        let c = inode_for_issue("PROJ", "PROJ-2");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, inode_for_project("PROJ"));
    }
}
