use super::*;

#[test]
fn winsize_uses_cell_metrics() {
    let size = WinSize::from(PtySize {
        cols: 80,
        rows: 24,
        cell_width: 9.0,
        cell_height: 18.0,
    });
    assert_eq!(size.cols, 80);
    assert_eq!(size.rows, 24);
    assert_eq!(size.pixel_width, 720);
    assert_eq!(size.pixel_height, 432);
}

#[test]
fn writer_control_coalesces_to_the_latest_resize() {
    let (writer, receiver) = WriterHandle::channel();
    let first = WinSize::from(test_size());
    let latest = WinSize::from(PtySize {
        cols: 132,
        rows: 43,
        ..test_size()
    });

    writer.resize(first).expect("first resize should queue");
    writer.resize(latest).expect("latest resize should queue");
    assert_eq!(
        receiver.try_iter().count(),
        1,
        "all pending resize work should share one wake"
    );
    let mut applied = Vec::new();
    assert!(service_writer_control(
        &(),
        &writer.control,
        &mut |_, window_size| {
            applied.push(window_size);
            Ok(())
        }
    ));

    assert_eq!(applied, vec![latest]);
}

#[test]
fn writer_control_reapplies_a_same_size_nudge() {
    let (writer, _receiver) = WriterHandle::channel();
    let window_size = WinSize::from(test_size());
    let mut applied = Vec::new();

    for _ in 0..2 {
        writer
            .resize(window_size)
            .expect("same-size resize should queue");
        assert!(service_writer_control(
            &(),
            &writer.control,
            &mut |_, applied_size| {
                applied.push(applied_size);
                Ok(())
            }
        ));
    }

    assert_eq!(applied, vec![window_size, window_size]);
}

#[test]
fn writer_control_close_is_sticky_and_cancels_pending_work() {
    let (writer, receiver) = WriterHandle::channel();
    writer
        .resize(WinSize::from(test_size()))
        .expect("resize should queue before close");
    writer.close();

    let mut resize_called = false;
    assert!(!service_writer_control(
        &(),
        &writer.control,
        &mut |_, _| {
            resize_called = true;
            Ok(())
        }
    ));
    assert!(!resize_called, "close should cancel the pending resize");
    assert!(writer.control.is_closed());
    assert_eq!(
        writer
            .write_owned(b"discarded".to_vec())
            .unwrap_err()
            .kind(),
        ErrorKind::BrokenPipe
    );
    assert_eq!(
        writer
            .resize(WinSize::from(test_size()))
            .unwrap_err()
            .kind(),
        ErrorKind::BrokenPipe
    );
    assert!(
        matches!(
            receiver.recv_timeout(Duration::from_millis(100)),
            Ok(WriterCommand::Wake(_))
        ),
        "resize and close should share one wake for an idle receiver"
    );
    assert!(
        receiver.try_recv().is_err(),
        "close must not add another wake"
    );
}

#[test]
fn writer_wake_reservation_clears_when_receiver_drops() {
    let (writer, receiver) = WriterHandle::channel();
    writer
        .resize(WinSize::from(test_size()))
        .expect("resize should reserve one wake");
    assert!(writer.control.wake_pending.load(Ordering::Acquire));

    drop(receiver);
    assert!(!writer.control.wake_pending.load(Ordering::Acquire));
    assert_eq!(
        writer
            .resize(WinSize::from(test_size()))
            .expect_err("a fresh wake should observe the disconnected receiver")
            .kind(),
        ErrorKind::BrokenPipe
    );
    assert!(writer.control.is_closed());
}

#[test]
fn writer_backlog_enforces_byte_and_entry_limits_until_reservations_drop() {
    let (writer, receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: 5,
        max_entries: 2,
    });
    writer
        .write_owned(b"abc".to_vec())
        .expect("first write should reserve bytes");
    writer
        .write_owned(b"de".to_vec())
        .expect("second write should fill both limits");

    let byte_error = writer
        .write_owned(b"f".to_vec())
        .expect_err("byte-full backlog should reject another write");
    assert_eq!(byte_error.kind(), ErrorKind::WouldBlock);
    assert!(byte_error.to_string().contains("backlog is full"));
    assert_eq!(writer.control.backlog(), (5, 2));

    let first = receiver.recv().expect("first write should remain queued");
    assert_eq!(
        writer.control.backlog(),
        (5, 2),
        "a dequeued but unfinished write must remain reserved"
    );
    drop(first);
    assert_eq!(writer.control.backlog(), (2, 1));
    drop(receiver);
    assert_eq!(
        writer.control.backlog(),
        (0, 0),
        "receiver teardown should release every queued reservation"
    );

    let (writer, receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: 10,
        max_entries: 1,
    });
    writer
        .write_owned(b"a".to_vec())
        .expect("first entry should queue");
    let entry_error = writer
        .write_owned(b"b".to_vec())
        .expect_err("entry-full backlog should reject another write");
    assert_eq!(entry_error.kind(), ErrorKind::WouldBlock);
    assert_eq!(writer.control.backlog(), (1, 1));
    drop(receiver);
    assert_eq!(writer.control.backlog(), (0, 0));
}

#[test]
fn owned_writer_payload_accounts_for_spare_vec_capacity() {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(b"tiny");
    let retained_capacity = bytes.capacity();
    assert!(retained_capacity > bytes.len());

    let (rejected, rejected_receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: retained_capacity - 1,
        max_entries: 1,
    });
    let error = rejected
        .write_owned(bytes)
        .expect_err("spare owned capacity must count against the heap cap");
    assert_eq!(error.kind(), ErrorKind::WouldBlock);
    assert_eq!(rejected.control.backlog(), (0, 0));
    assert!(rejected_receiver.try_recv().is_err());

    let mut admitted_bytes = Vec::with_capacity(retained_capacity);
    admitted_bytes.extend_from_slice(b"tiny");
    assert_eq!(admitted_bytes.capacity(), retained_capacity);
    let (admitted, admitted_receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: retained_capacity,
        max_entries: 1,
    });
    admitted
        .write_owned(admitted_bytes)
        .expect("the exact retained allocation should be admitted");
    assert_eq!(admitted.control.backlog(), (retained_capacity, 1));
    let active = admitted_receiver
        .recv()
        .expect("accepted owned write should remain queued");
    assert_eq!(admitted.control.backlog(), (retained_capacity, 1));
    drop(active);
    assert_eq!(admitted.control.backlog(), (0, 0));
}

#[test]
fn protocol_reply_lane_survives_normal_saturation_and_is_bounded() {
    let (writer, receiver) = WriterHandle::channel_with_all_limits(
        WriterLimits {
            max_bytes: 1,
            max_entries: 1,
        },
        WriterLimits {
            max_bytes: 5,
            max_entries: 1,
        },
    );
    writer
        .write_owned(vec![b'n'])
        .expect("normal lane should fill");
    writer
        .write_protocol_reply_owned(b"reply".to_vec())
        .expect("normal saturation must not reject a protocol reply");
    assert_eq!(writer.control.backlog(), (1, 1));
    assert_eq!(writer.control.protocol_reply_backlog(), (5, 1));

    let error = writer
        .write_protocol_reply_owned(b"x".to_vec())
        .expect_err("the independent reply lane must remain bounded");
    assert_eq!(error.kind(), ErrorKind::WouldBlock);
    assert_eq!(writer.control.protocol_reply_backlog(), (5, 1));

    let active_reply = writer
        .protocol_replies
        .pop()
        .expect("accepted reply should remain reserved while active");
    writer.close();
    assert_eq!(writer.control.protocol_reply_backlog(), (5, 1));
    drop(active_reply);
    assert_eq!(writer.control.protocol_reply_backlog(), (0, 0));
    drop(receiver);
    drop(writer);

    let (queued, _queued_receiver) = WriterHandle::channel_with_all_limits(
        WriterLimits {
            max_bytes: 1,
            max_entries: 1,
        },
        WriterLimits {
            max_bytes: 8,
            max_entries: 1,
        },
    );
    queued
        .write_protocol_reply_owned(b"reply".to_vec())
        .expect("reply should queue before close");
    assert_eq!(queued.control.protocol_reply_backlog(), (5, 1));
    queued.close();
    assert_eq!(queued.control.protocol_reply_backlog(), (0, 0));

    let (disconnected, disconnected_receiver) = WriterHandle::channel_with_all_limits(
        WriterLimits {
            max_bytes: 1,
            max_entries: 1,
        },
        WriterLimits {
            max_bytes: 8,
            max_entries: 1,
        },
    );
    let disconnected_control = disconnected.control.clone();
    drop(disconnected_receiver);
    let error = disconnected
        .write_protocol_reply_owned(b"reply".to_vec())
        .expect_err("a disconnected writer must reject a queued reply");
    assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    assert!(disconnected_control.is_closed());
    drop(disconnected);
    assert_eq!(disconnected_control.protocol_reply_backlog(), (0, 0));
}

#[test]
fn unqueueable_reader_reply_is_dropped_and_keeps_the_writer_open() {
    let (writer, _receiver) = WriterHandle::channel_with_all_limits(
        WriterLimits {
            max_bytes: 1,
            max_entries: 1,
        },
        WriterLimits {
            max_bytes: 0,
            max_entries: 0,
        },
    );
    queue_reader_protocol_reply(&writer, b"reply".to_vec());
    assert!(!writer.control.is_closed());
    assert_eq!(writer.control.protocol_reply_backlog(), (0, 0));
    writer.close();
    queue_reader_protocol_reply(&writer, b"reply".to_vec());
    assert_eq!(writer.control.protocol_reply_backlog(), (0, 0));
}

#[test]
fn writer_backlog_releases_failed_sends_and_close_cancellation() {
    let (writer, receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: 16,
        max_entries: 4,
    });
    drop(receiver);
    assert_eq!(
        writer
            .write_owned(b"disconnected".to_vec())
            .expect_err("disconnected send should fail")
            .kind(),
        ErrorKind::BrokenPipe
    );
    assert_eq!(writer.control.backlog(), (0, 0));

    let (writer, receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: 16,
        max_entries: 4,
    });
    writer
        .write_owned(b"queued".to_vec())
        .expect("write should queue before close");
    let control = writer.control.clone();
    let protocol_replies = writer.protocol_replies.clone();
    writer.close();
    let _ = run_pty_writer_loop(
        PartialWriter {
            max_write: usize::MAX,
            ..PartialWriter::default()
        },
        receiver,
        control.clone(),
        protocol_replies,
        |_| Ok(()),
        |_, _| Ok(()),
    );
    assert_eq!(
        control.backlog(),
        (0, 0),
        "close-driven loop teardown should release queued writes"
    );
}

#[test]
fn empty_writer_payloads_are_successful_without_channel_nodes() {
    let (writer, receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: 0,
        max_entries: 0,
    });
    writer
        .write_owned(Vec::new())
        .expect("empty write should bypass backlog admission");
    writer
        .write_owned(Vec::with_capacity(64))
        .expect("empty spare capacity should be dropped without a queue node");
    writer
        .write(&[])
        .expect("empty borrowed write should bypass backlog admission");
    assert!(receiver.try_recv().is_err());
    assert_eq!(writer.control.backlog(), (0, 0));

    writer.close();
    writer
        .write_owned(Vec::new())
        .expect("empty write should remain a no-op after close");
    writer
        .write_owned(Vec::with_capacity(64))
        .expect("empty spare capacity should remain a no-op after close");
    assert_eq!(
        receiver.try_iter().count(),
        1,
        "close should queue one wake"
    );
    assert_eq!(writer.control.backlog(), (0, 0));
}

#[test]
fn borrowed_writer_payload_is_admitted_before_copying() {
    let (writer, receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: 3,
        max_entries: 1,
    });
    let error = writer
        .write(b"four")
        .expect_err("oversized borrowed input should fail admission");
    assert_eq!(error.kind(), ErrorKind::WouldBlock);
    assert_eq!(writer.control.backlog(), (0, 0));
    assert!(receiver.try_recv().is_err());

    writer
        .write(b"fit")
        .expect("admitted borrowed input should queue after reservation");
    assert_eq!(writer.control.backlog(), (3, 1));
    drop(receiver);
    assert_eq!(writer.control.backlog(), (0, 0));
}

#[test]
fn writer_loop_keeps_partial_writes_fifo() {
    let (writer, receiver) = WriterHandle::channel();
    let control = writer.control.clone();
    let observed_control = control.clone();
    let first = b"AAAAA".to_vec();
    let second = b"BBBBB".to_vec();
    writer
        .write_owned(first.clone())
        .expect("first write should queue");
    writer
        .write_owned(second.clone())
        .expect("second write should queue");
    let protocol_replies = writer.protocol_replies.clone();
    drop(writer);

    let sink = run_pty_writer_loop(
        PartialWriter {
            max_write: 2,
            ..PartialWriter::default()
        },
        receiver,
        control,
        protocol_replies,
        |_| Ok(()),
        |_, _| Ok(()),
    );

    let mut expected = first;
    expected.extend_from_slice(&second);
    assert_eq!(sink.bytes, expected);
    assert_eq!(sink.write_calls, 6, "both writes should be partial");
    assert_eq!(observed_control.backlog(), (0, 0));
}

#[test]
fn protocol_reply_preempts_a_blocked_normal_write_then_normal_fifo_resumes() {
    let (writer, receiver) = WriterHandle::channel();
    writer
        .write_owned(b"normal-one".to_vec())
        .expect("first normal write should queue");
    writer
        .write_owned(b"normal-two".to_vec())
        .expect("second normal write should queue");
    let control = writer.control.clone();
    let protocol_replies = writer.protocol_replies.clone();
    let mut reply_writer = Some(writer);

    let sink = run_pty_writer_loop(
        WouldBlockOnceWriter::default(),
        receiver,
        control.clone(),
        protocol_replies,
        |_| {
            let writer = reply_writer
                .take()
                .expect("the first blocked write should enqueue one reply");
            writer
                .write_protocol_reply_owned(b"reply".to_vec())
                .expect("priority reply should queue outside the normal budget");
            drop(writer);
            Ok(())
        },
        |_, _| Ok(()),
    );

    assert_eq!(sink.bytes, b"replynormal-onenormal-two");
    assert_eq!(control.backlog(), (0, 0));
    assert_eq!(control.protocol_reply_backlog(), (0, 0));
}

#[test]
fn protocol_reply_waits_for_a_partial_write_to_finish() {
    let (writer, receiver) = WriterHandle::channel();
    writer
        .write_owned(b"normal-one".to_vec())
        .expect("first normal write should queue");
    writer
        .write_owned(b"normal-two".to_vec())
        .expect("second normal write should queue");
    let control = writer.control.clone();
    let protocol_replies = writer.protocol_replies.clone();
    let mut reply_writer = Some(writer);

    let sink = run_pty_writer_loop(
        PartialThenWouldBlockWriter::default(),
        receiver,
        control.clone(),
        protocol_replies,
        |_| {
            let writer = reply_writer
                .take()
                .expect("the blocked partial write should enqueue one reply");
            writer
                .write_protocol_reply_owned(b"reply".to_vec())
                .expect("priority reply should queue outside the normal budget");
            drop(writer);
            Ok(())
        },
        |_, _| Ok(()),
    );

    assert_eq!(sink.bytes, b"normal-onereplynormal-two");
    assert_eq!(control.backlog(), (0, 0));
    assert_eq!(control.protocol_reply_backlog(), (0, 0));
}

#[test]
fn unqueueable_live_reader_reply_is_dropped_without_ending_the_session() {
    const MARKER: &[u8] = b"TERMY_NATIVE_OVERSIZED_REPLY_READY";

    let (exit_tx, exit_rx) = mpsc::sync_channel(1);
    let (echo_tx, echo_rx) = mpsc::sync_channel(1);
    let mut output = Vec::new();
    let mut replied = false;
    let terminal = Transport::spawn(
        SpawnConfig {
            program: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "printf TERMY_NATIVE_OVERSIZED_REPLY_READY; read line; printf \"got:$line\""
                    .to_string(),
            ],
            working_directory: None,
            environment: Vec::new(),
        },
        test_size(),
        move |bytes| {
            output.extend_from_slice(bytes);
            if output.windows(4).any(|part| part == b"got:") {
                let _ = echo_tx.try_send(());
            }
            if !replied && output.windows(MARKER.len()).any(|part| part == MARKER) {
                replied = true;
                vec![b'x'; MAX_PROTOCOL_REPLY_BACKLOG_BYTES + 1]
            } else {
                Vec::new()
            }
        },
        move || {
            let _ = exit_tx.send(());
        },
    )
    .expect("PTY should start before the oversized reply is generated");

    assert!(
        exit_rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "an unqueueable reply must not end the session"
    );
    terminal
        .write_owned(b"still-alive\n".to_vec())
        .expect("input should still reach the child");
    echo_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("child should keep reading input");
    exit_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("child should exit normally");
    drop(terminal);
}

#[test]
fn mouse_motion_fills_only_half_the_backlog_and_leaves_room_for_keys() {
    let (writer, _receiver) = WriterHandle::channel_with_limits(WriterLimits {
        max_bytes: 8,
        max_entries: 4,
    });
    writer.write_motion_owned(b"m1".to_vec()).unwrap();
    writer.write_motion_owned(b"m2".to_vec()).unwrap();
    let dropped = writer
        .write_motion_owned(b"m3".to_vec())
        .expect_err("motion past half the backlog is dropped");
    assert_eq!(dropped.kind(), ErrorKind::WouldBlock);
    assert_eq!(writer.control.backlog(), (4, 2));
    writer.write_owned(b"k1".to_vec()).unwrap();
    writer.write_owned(b"k2".to_vec()).unwrap();
    assert_eq!(writer.control.backlog(), (8, 4));
}

#[test]
fn waiting_on_a_hung_up_descriptor_fails_instead_of_spinning() {
    // Close-on-exec, so a child that another test forks meanwhile doesn't
    // keep the read end open.
    let (read, write) = io::pipe().unwrap();
    let write = File::from(std::os::fd::OwnedFd::from(write));
    let (wake, _wake_sender) = signal_socket_pair().unwrap();
    drop(read);
    let started = Instant::now();
    let result = wait_for_pty_writable(&write, &wake);
    assert!(started.elapsed() < Duration::from_millis(500), "{result:?}");
    assert_eq!(result.unwrap_err().kind(), ErrorKind::BrokenPipe);
}

#[test]
fn a_failed_resize_keeps_the_writer_running() {
    let (writer, receiver) = WriterHandle::channel();
    let control = writer.control.clone();
    writer
        .resize(WinSize::from(test_size()))
        .expect("resize should queue");
    writer
        .write_owned(b"after".to_vec())
        .expect("write should queue");
    let protocol_replies = writer.protocol_replies.clone();
    drop(writer);

    let mut resizes = 0;
    let sink = run_pty_writer_loop(
        PartialWriter {
            max_write: usize::MAX,
            ..PartialWriter::default()
        },
        receiver,
        control,
        protocol_replies,
        |_| Ok(()),
        |_, _| {
            resizes += 1;
            Err(io::Error::from_raw_os_error(25))
        },
    );
    assert_eq!(resizes, 1);
    assert_eq!(sink.bytes, b"after");
}
