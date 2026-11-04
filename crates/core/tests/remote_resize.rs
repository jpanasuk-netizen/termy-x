use std::sync::{Arc, Mutex};
use termy_core::{
    Terminal, TerminalEvent, TerminalSize,
    remote::{
        self, RemoteCommand, RemoteHostReply, RemoteHostRequest, RemoteReply, RemoteState,
        RemoteTransport,
    },
};

/// Queues commands like the session client's input thread. The displayed
/// frame changes only when the test publishes the host's state.
struct QueuedHost {
    terminal: Mutex<Terminal>,
    queued: Mutex<Vec<RemoteCommand>>,
    displayed: Mutex<Arc<RemoteState>>,
}

impl QueuedHost {
    fn apply_queued(&self) {
        let mut terminal = self.terminal.lock().unwrap();
        for command in self.queued.lock().unwrap().drain(..) {
            remote::execute(&mut terminal, command);
        }
    }
    fn publish(&self) {
        *self.displayed.lock().unwrap() =
            Arc::new(RemoteState::capture(&self.terminal.lock().unwrap()));
    }
    fn size(&self) -> TerminalSize {
        self.terminal.lock().unwrap().size()
    }
}

impl RemoteTransport for QueuedHost {
    fn state(&self) -> Arc<RemoteState> {
        Arc::clone(&self.displayed.lock().unwrap())
    }
    fn request(&self, command: RemoteCommand) -> anyhow::Result<RemoteReply> {
        Ok(remote::execute(&mut self.terminal.lock().unwrap(), command))
    }
    fn send(&self, command: RemoteCommand) {
        self.queued.lock().unwrap().push(command);
    }
    fn take_events(&self) -> Vec<TerminalEvent> {
        Vec::new()
    }
    fn take_host_requests(&self) -> Vec<(u64, RemoteHostRequest)> {
        Vec::new()
    }
    fn reply_to_host(&self, _: u64, _: RemoteHostReply) {}
    fn has_pending_events(&self) -> bool {
        false
    }
    fn set_wakeup_enabled(&self, _: bool) {}
}

fn size(cols: u16, rows: u16) -> TerminalSize {
    TerminalSize {
        cols,
        rows,
        ..TerminalSize::default()
    }
}

fn fixture(initial: TerminalSize) -> (Arc<QueuedHost>, Terminal) {
    let terminal = Terminal::new_display(initial, None);
    let host = Arc::new(QueuedHost {
        displayed: Mutex::new(Arc::new(RemoteState::capture(&terminal))),
        terminal: Mutex::new(terminal),
        queued: Mutex::new(Vec::new()),
    });
    (Arc::clone(&host), Terminal::from_remote(host))
}

/// Mirrors the desktop resize decision: resize only when the size differs.
fn fit(terminal: &mut Terminal, target: TerminalSize) {
    if terminal.requested_size() != target {
        terminal.resize(target);
    }
}

#[test]
fn reattached_pane_returns_to_its_size_after_an_intermediate_resize() {
    // The host still holds the program at the size it had before restart.
    let restored = size(120, 40);
    let (host, mut remote) = fixture(restored);

    // Window setup passes through another size before settling on the
    // restored layout. Neither resize has reached a published frame yet.
    fit(&mut remote, size(80, 24));
    assert_eq!(remote.size(), restored, "frames lag queued resizes");
    fit(&mut remote, restored);

    host.apply_queued();
    host.publish();
    assert_eq!(host.size(), restored);
    assert_eq!(remote.size(), restored);
    assert_eq!(remote.requested_size(), restored);
}

#[test]
fn requested_size_follows_host_after_its_frame_catches_up() {
    let (host, mut remote) = fixture(size(100, 30));
    fit(&mut remote, size(90, 20));
    assert_eq!(remote.requested_size(), size(90, 20));
    host.apply_queued();
    host.publish();
    assert_eq!(remote.requested_size(), size(90, 20));

    // Another client resizes the shared session afterward. The view must see
    // that change so its next layout pass can claim the pane again.
    host.terminal.lock().unwrap().resize(size(70, 10));
    host.publish();
    assert_eq!(remote.requested_size(), size(70, 10));
    fit(&mut remote, size(90, 20));
    host.apply_queued();
    assert_eq!(host.size(), size(90, 20));
}
