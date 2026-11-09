//! Display terminals as replicas: full-state snapshots, replies returned in
//! order from the feed, and a replica mode that never answers queries.

use termy_core::ffi::{
    TermyFfiBytes, TermyFfiStatus, TermyFfiTerminal, termy_buffer_free, termy_display_terminal_new,
    termy_size_default, termy_terminal_feed_output_with_replies, termy_terminal_free,
    termy_terminal_set_protocol_replies_enabled, termy_terminal_snapshot_decode,
    termy_terminal_snapshot_encode,
};

struct Terminal(*mut TermyFfiTerminal);

impl Terminal {
    fn display() -> Self {
        let mut terminal = std::ptr::null_mut();
        let status = unsafe { termy_display_terminal_new(termy_size_default(), &mut terminal) };
        assert_eq!(status, TermyFfiStatus::Ok);
        Self(terminal)
    }

    fn feed(&self, bytes: &[u8]) -> Vec<u8> {
        let mut replies = TermyFfiBytes::default();
        let status = unsafe {
            termy_terminal_feed_output_with_replies(
                self.0,
                bytes.as_ptr(),
                bytes.len(),
                &mut replies,
            )
        };
        assert_eq!(status, TermyFfiStatus::Ok);
        take(replies)
    }

    fn snapshot(&self) -> Vec<u8> {
        let mut bytes = TermyFfiBytes::default();
        assert_eq!(
            unsafe { termy_terminal_snapshot_encode(self.0, &mut bytes) },
            TermyFfiStatus::Ok
        );
        take(bytes)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { termy_terminal_free(self.0) };
    }
}

fn take(bytes: TermyFfiBytes) -> Vec<u8> {
    if bytes.ptr.is_null() {
        return Vec::new();
    }
    let out = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) }.to_vec();
    unsafe { termy_buffer_free(bytes) };
    out
}

#[test]
fn replies_return_from_the_feed_in_order() {
    let terminal = Terminal::display();
    let replies = String::from_utf8(terminal.feed(b"\x1b[5n\x1b[c\x1b[6n")).unwrap();
    let status = replies.find("\x1b[0n").expect("status report");
    let attributes = replies.find("\x1b[?").expect("device attributes");
    let position = replies.find("\x1b[1;1R").expect("cursor position");
    assert!(status < attributes && attributes < position, "{replies:?}");
    assert!(terminal.feed(b"plain").is_empty());
}

#[test]
fn replicas_drop_replies_without_changing_state() {
    let server = Terminal::display();
    let replica = Terminal::display();
    assert_eq!(
        unsafe { termy_terminal_set_protocol_replies_enabled(replica.0, false) },
        TermyFfiStatus::Ok
    );
    let stream = b"text\x1b[5n\x1b]10;?\x07\x1b[c\x1b[?2004h more";
    assert!(!server.feed(stream).is_empty());
    assert!(replica.feed(stream).is_empty());
    assert_eq!(replica.snapshot(), server.snapshot());
}

#[test]
fn snapshot_decode_continues_like_the_original() {
    let server = Terminal::display();
    server.feed(
        "\x1b[1mbold\x1b[0m \u{4e2d}\r\n\x1b]2;title\x07\x1b[?1049h\x1b]8;;http://x\x07".as_bytes(),
    );
    let snapshot = server.snapshot();
    let mut decoded = std::ptr::null_mut();
    assert_eq!(
        unsafe { termy_terminal_snapshot_decode(snapshot.as_ptr(), snapshot.len(), &mut decoded) },
        TermyFfiStatus::Ok
    );
    let replica = Terminal(decoded);
    unsafe { termy_terminal_set_protocol_replies_enabled(replica.0, false) };
    let tail = b"link\x1b]8;;\x07\x1b[?1049l\x1b[5n after";
    assert!(!server.feed(tail).is_empty());
    assert!(replica.feed(tail).is_empty());
    assert_eq!(replica.snapshot(), server.snapshot());

    let mut rejected = std::ptr::null_mut();
    assert_eq!(
        unsafe { termy_terminal_snapshot_decode(b"garbage".as_ptr(), 7, &mut rejected) },
        TermyFfiStatus::InvalidSnapshot
    );
    assert!(rejected.is_null());
}
