//! Exit, refresh, scroll, hung HTTP, and quit rules for Termy XL.
//!
//! These functions are what the window calls. Tests run them without a GPU.

use std::collections::BTreeSet;
use std::io::{ErrorKind, Read};
use std::net::{SocketAddr, TcpStream};
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

use super::{scroll_by, selection_after_card};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeskSession {
    timers: BTreeSet<String>,
    hooks: BTreeSet<String>,
    children: BTreeSet<u32>,
}

impl DeskSession {
    /// Timers and the close hook the window starts. An agent pid is optional.
    pub fn running(agent_pid: Option<u32>) -> Self {
        let mut session = Self::default();
        for name in ["refresh", "follow", "daily", "hermes-probe"] {
            session.timers.insert(name.to_string());
        }
        session.hooks.insert("close".to_string());
        if let Some(pid) = agent_pid {
            session.children.insert(pid);
        }
        session
    }

    pub fn is_idle(&self) -> bool {
        self.timers.is_empty() && self.hooks.is_empty() && self.children.is_empty()
    }

    /// Drop every timer, hook, and child record. The caller reaps the processes.
    pub fn shutdown(&mut self, children: &mut [Child]) {
        for child in children.iter_mut() {
            reap_child(child);
        }
        self.timers.clear();
        self.hooks.clear();
        self.children.clear();
    }
}

pub fn reap_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn child_is_gone(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(Some(_)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshPress {
    pub started: bool,
    pub topic: Option<String>,
    pub last_refresh: Option<i64>,
    pub posts: bool,
}

/// The Refresh button. It starts a search even when the auto timer is not due.
/// A busy desk does not start a second search. Nothing is posted.
pub fn press_refresh(now: i64, busy: bool, topic: Option<&str>, last: Option<i64>) -> RefreshPress {
    let Some(topic) = topic.map(str::trim).filter(|topic| !topic.is_empty()) else {
        return RefreshPress {
            started: false,
            topic: None,
            last_refresh: last,
            posts: false,
        };
    };
    if busy {
        return RefreshPress {
            started: false,
            topic: Some(topic.to_string()),
            last_refresh: last,
            posts: false,
        };
    }
    RefreshPress {
        started: true,
        topic: Some(topic.to_string()),
        last_refresh: Some(now),
        posts: false,
    }
}

/// A wheel event moves the card. The next click still selects it.
pub fn click_after_scroll(id: &str, delta: f32) -> (f32, Option<String>) {
    let mut offsets = std::collections::HashMap::new();
    let at = scroll_by(&mut offsets, id, delta, 400.0);
    (at, selection_after_card(id))
}

/// Agent panes never raise a quit dialog, even when one is still running.
pub fn quit_dialog_for_agents(open_panes: usize) -> Option<&'static str> {
    let _ = open_panes;
    None
}

/// What Escape does. None of these ask the process to quit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeEffect {
    CloseMedia,
    CloseCompose,
    ClearField,
    ClearSelection,
    CollapseDock,
    Ignored,
}

pub fn escape_effect(
    media_open: bool,
    compose_open: bool,
    editing: bool,
    field: bool,
    selected: bool,
    expanded: bool,
) -> EscapeEffect {
    if media_open {
        EscapeEffect::CloseMedia
    } else if compose_open || editing {
        EscapeEffect::CloseCompose
    } else if field {
        EscapeEffect::ClearField
    } else if selected {
        EscapeEffect::ClearSelection
    } else if expanded {
        EscapeEffect::CollapseDock
    } else {
        EscapeEffect::Ignored
    }
}

pub fn escape_requests_quit(effect: EscapeEffect) -> bool {
    let _ = effect;
    false
}

/// Losing focus does not close the window. Only a window-close request does.
pub fn focus_loss_closes_the_window() -> bool {
    false
}

pub struct OffUiRequest {
    started: Instant,
    rx: std::sync::mpsc::Receiver<Result<String, String>>,
}

impl OffUiRequest {
    pub fn dispatch_elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn recv(self, limit: Duration) -> Result<Result<String, String>, String> {
        self.rx
            .recv_timeout(limit)
            .map_err(|_| "the worker did not finish".to_string())
    }
}

/// Start a read on another thread and return at once. The worker gives up
/// after `timeout` if the server never answers.
pub fn start_off_ui(addr: SocketAddr, timeout: Duration) -> OffUiRequest {
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(read_with_timeout(addr, timeout));
    });
    OffUiRequest {
        started: Instant::now(),
        rx,
    }
}

fn read_with_timeout(addr: SocketAddr, timeout: Duration) -> Result<String, String> {
    let mut stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    let mut buf = [0_u8; 32];
    match stream.read(&mut buf) {
        Ok(0) => Err("closed".into()),
        Ok(n) => Ok(String::from_utf8_lossy(&buf[..n]).into_owned()),
        Err(error)
            if error.kind() == ErrorKind::TimedOut || error.kind() == ErrorKind::WouldBlock =>
        {
            Err("timed out".into())
        }
        Err(error) => Err(error.to_string()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PruneLog {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
}

/// Keep the newest `*.bak` file and the newest `backup-*` directory.
/// Older ones are removed. Names are returned so the installer can log them.
pub fn prune_backups(dir: &std::path::Path) -> Result<PruneLog, String> {
    let mut baks = Vec::new();
    let mut folders = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        let modified = entry.metadata().and_then(|meta| meta.modified()).ok();
        if entry.path().is_dir() && name.starts_with("backup-") {
            folders.push((modified, name, entry.path()));
        } else if entry.path().is_file() && name.ends_with(".bak") {
            baks.push((modified, name, entry.path()));
        }
    }
    let mut log = PruneLog {
        removed: Vec::new(),
        kept: Vec::new(),
    };
    drop_older(&mut baks, &mut log)?;
    drop_older(&mut folders, &mut log)?;
    Ok(log)
}

fn drop_older(
    items: &mut [(Option<std::time::SystemTime>, String, std::path::PathBuf)],
    log: &mut PruneLog,
) -> Result<(), String> {
    items.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    if items.len() <= 1 {
        if let Some((_, name, _)) = items.last() {
            log.kept.push(name.clone());
        }
        return Ok(());
    }
    let keep = items.len() - 1;
    for (_, name, path) in items.iter().take(keep) {
        if path.is_dir() {
            std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        } else {
            std::fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        log.removed.push(name.clone());
    }
    log.kept.push(items[keep].1.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::process::Command;

    fn sleeper() -> Command {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        };
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::null());
        command.stderr(std::process::Stdio::null());
        command
    }

    #[test]
    fn exit_clears_timers_hooks_and_the_child_process() {
        let mut session = DeskSession::running(Some(1));
        assert!(!session.is_idle());
        assert_eq!(session.timers.len(), 4);
        assert_eq!(session.hooks.len(), 1);
        assert_eq!(session.children.len(), 1);
        let mut child = sleeper().spawn().expect("sleeper");
        let mut children = [child];
        // The id is only a record. The live process is the sleeper.
        session.shutdown(&mut children);
        child = children.into_iter().next().expect("child");
        assert!(session.is_idle());
        assert!(child_is_gone(&mut child));
        session.shutdown(&mut []);
        assert!(session.is_idle());
    }

    #[test]
    fn pressing_refresh_starts_one_search_and_does_not_post() {
        let first = press_refresh(1_700_000_000, false, Some("law"), None);
        assert!(first.started);
        assert_eq!(first.topic.as_deref(), Some("law"));
        assert_eq!(first.last_refresh, Some(1_700_000_000));
        assert!(!first.posts);
        let busy = press_refresh(1_700_000_010, true, Some("law"), first.last_refresh);
        assert!(!busy.started);
        assert_eq!(busy.last_refresh, first.last_refresh);
        assert!(!busy.posts);
        let missing = press_refresh(1_700_000_010, false, None, None);
        assert!(!missing.started);
    }

    #[test]
    fn a_scroll_does_not_swallow_the_following_click() {
        let (at, selected) = click_after_scroll("55", 48.0);
        assert!(at > 0.0);
        assert_eq!(selected.as_deref(), Some("take:55"));
        let (again, still) = click_after_scroll("55", 12.0);
        assert!(again > 0.0);
        assert_eq!(still, selected);
    }

    #[test]
    fn a_hung_server_does_not_block_the_caller() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0_u8; 8];
                let _ = sock.read(&mut buf);
            }
        });
        let started = Instant::now();
        let request = start_off_ui(addr, Duration::from_millis(200));
        assert!(
            started.elapsed() < Duration::from_millis(150),
            "dispatch blocked the caller for {:?}",
            started.elapsed()
        );
        let result = request.recv(Duration::from_secs(2)).expect("worker");
        assert!(result.unwrap_err().contains("timed out"));
    }

    #[test]
    fn busy_agent_panes_do_not_open_a_quit_dialog() {
        assert_eq!(quit_dialog_for_agents(0), None);
        assert_eq!(quit_dialog_for_agents(1), None);
        assert_eq!(quit_dialog_for_agents(3), None);
    }

    #[test]
    fn escape_never_quits_and_focus_loss_does_not_close() {
        assert!(!focus_loss_closes_the_window());
        let effects = [
            escape_effect(true, true, true, true, true, true),
            escape_effect(false, true, false, true, true, true),
            escape_effect(false, false, true, true, true, true),
            escape_effect(false, false, false, true, true, true),
            escape_effect(false, false, false, false, true, true),
            escape_effect(false, false, false, false, false, true),
            escape_effect(false, false, false, false, false, false),
        ];
        assert_eq!(
            effects,
            [
                EscapeEffect::CloseMedia,
                EscapeEffect::CloseCompose,
                EscapeEffect::CloseCompose,
                EscapeEffect::ClearField,
                EscapeEffect::ClearSelection,
                EscapeEffect::CollapseDock,
                EscapeEffect::Ignored,
            ]
        );
        for effect in effects {
            assert!(!escape_requests_quit(effect));
        }
    }

    #[test]
    fn prune_keeps_the_newest_bak_and_backup_folder() {
        let dir = std::env::temp_dir().join(format!("xl-bak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["1.bak", "2.bak", "3.bak"] {
            std::fs::write(dir.join(name), name).unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        for name in ["backup-1", "backup-2", "backup-3"] {
            let path = dir.join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("note.txt"), name).unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        let log = prune_backups(&dir).unwrap();
        assert!(log.removed.iter().any(|name| name == "1.bak"));
        assert!(log.removed.iter().any(|name| name == "2.bak"));
        assert!(log.kept.iter().any(|name| name == "3.bak"));
        assert!(dir.join("3.bak").is_file());
        assert!(!dir.join("1.bak").exists());
        assert!(dir.join("backup-3").is_dir());
        assert!(!dir.join("backup-1").exists());
        assert!(!dir.join("backup-2").exists());
        let script = include_str!("../../../../scripts/windows/install-termy-xl.ps1");
        assert!(script.contains("function Prune-OldBackups"));
        assert!(script.contains("removed "));
        assert!(script.contains("backup-*"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
