use std::io::{BufRead, BufReader, Read, Write};

use flume::{Receiver, Sender, TrySendError};

use super::super::command::{
    command_with_completion_token, is_control_completion_token, split_control_completion_token,
};
use super::super::types::{TmuxControlError, TmuxNotification};

use super::{
    channel::{
        ActiveControlCommand, ControlRequest, PendingCommand, TrackedPendingCommand,
        append_command_output_chunk, claim_pending_for_command_begin, complete_pending_command,
        map_command_completion_response,
    },
    coalescer::{NotificationCoalescer, flush_notification_coalescer, signal_fatal_exit},
    parser::{ControlStateEvent, ControlStateMachine},
};

#[allow(clippy::too_many_arguments)]
pub fn spawn_control_threads<W, R>(
    child: Option<std::process::Child>,
    child_stdin: W,
    child_stdout: R,
    request_rx: Receiver<ControlRequest>,
    pending_tx: Sender<PendingCommand>,
    pending_rx: Receiver<PendingCommand>,
    notifications_tx: Sender<TmuxNotification>,
    fatal_exit_tx: Sender<Option<String>>,
    event_wakeup_tx: Option<Sender<()>>,
) where
    W: Write + Send + 'static,
    R: Read + Send + 'static,
{
    if let Some(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }

    std::thread::spawn(move || {
        let mut stdin = child_stdin;
        while let Ok(request) = request_rx.recv() {
            let command = request.command;
            let completion_token = request.completion_token;
            let response_tx = request.response_tx;
            let (completion_tx, completion_rx) = flume::bounded(1);
            let response_tx_for_write_error = response_tx.clone();
            let completion_tx_for_write_error = completion_tx.clone();
            let command_with_token =
                command_with_completion_token(command.as_str(), completion_token.as_str());

            match pending_tx.try_send(PendingCommand {
                command: command.clone(),
                completion_token,
                response_tx,
                completion_tx,
            }) {
                Ok(()) => {}
                Err(TrySendError::Full(pending)) => {
                    complete_pending_command(
                        pending,
                        Err(TmuxControlError::channel(
                            "tmux control pending queue is full",
                        )),
                    );
                    break;
                }
                Err(TrySendError::Disconnected(pending)) => {
                    complete_pending_command(
                        pending,
                        Err(TmuxControlError::channel(
                            "tmux control reader is unavailable",
                        )),
                    );
                    break;
                }
            }

            if stdin.write_all(command_with_token.as_bytes()).is_err() {
                if let Some(response_tx) = response_tx_for_write_error {
                    let _ = response_tx.send(Err(TmuxControlError::channel(
                        "failed to write command to tmux control stdin",
                    )));
                }
                let _ = completion_tx_for_write_error.send(());
                break;
            }
            if stdin.write_all(b"\n").is_err() {
                if let Some(response_tx) = response_tx_for_write_error {
                    let _ = response_tx.send(Err(TmuxControlError::channel(
                        "failed to write command terminator to tmux control stdin",
                    )));
                }
                let _ = completion_tx_for_write_error.send(());
                break;
            }
            if stdin.flush().is_err() {
                if let Some(response_tx) = response_tx_for_write_error {
                    let _ = response_tx.send(Err(TmuxControlError::channel(
                        "failed to flush tmux control stdin",
                    )));
                }
                let _ = completion_tx_for_write_error.send(());
                break;
            }
            if completion_rx.recv().is_err() {
                break;
            }
        }
    });

    std::thread::spawn(move || {
        let mut reader = BufReader::new(child_stdout);
        let mut line = Vec::<u8>::new();
        let mut current_command = None::<ActiveControlCommand>;
        let mut control_state = ControlStateMachine::default();
        let mut notifications = NotificationCoalescer::default();

        let fail_pending = |pending: PendingCommand, error: TmuxControlError| {
            complete_pending_command(pending, Err(error));
        };
        let fail_all_pending = |current_command: &mut Option<ActiveControlCommand>,
                                pending_rx: &Receiver<PendingCommand>,
                                error: TmuxControlError| {
            if let Some(active_command) = current_command.take()
                && let ActiveControlCommand::Tracked(tracked) = active_command
            {
                fail_pending(tracked.pending, error.clone());
            }
            while let Ok(pending) = pending_rx.try_recv() {
                fail_pending(pending, error.clone());
            }
        };
        let fail_with_exit = |current_command: &mut Option<ActiveControlCommand>,
                              pending_rx: &Receiver<PendingCommand>,
                              notifications_tx: &Sender<TmuxNotification>,
                              fatal_exit_tx: &Sender<Option<String>>,
                              notifications: &mut NotificationCoalescer,
                              event_wakeup_tx: Option<&Sender<()>>,
                              error: TmuxControlError| {
            let exit_reason = Some(error.message.clone());
            fail_all_pending(current_command, pending_rx, error);
            signal_fatal_exit(fatal_exit_tx, exit_reason.clone());
            notifications.push(TmuxNotification::Exit(exit_reason));
            let _ = flush_notification_coalescer(notifications, notifications_tx, event_wakeup_tx);
        };

        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line);
            let Ok(read) = read else {
                let exit_reason = Some("tmux control mode read failure".to_string());
                fail_all_pending(
                    &mut current_command,
                    &pending_rx,
                    TmuxControlError::channel("tmux control mode read failure"),
                );
                signal_fatal_exit(&fatal_exit_tx, exit_reason.clone());
                notifications.push(TmuxNotification::Exit(exit_reason));
                let _ = flush_notification_coalescer(
                    &mut notifications,
                    &notifications_tx,
                    event_wakeup_tx.as_ref(),
                );
                break;
            };

            if read == 0 {
                fail_all_pending(
                    &mut current_command,
                    &pending_rx,
                    TmuxControlError::channel("tmux control mode closed"),
                );
                signal_fatal_exit(&fatal_exit_tx, None);
                notifications.push(TmuxNotification::Exit(None));
                let _ = flush_notification_coalescer(
                    &mut notifications,
                    &notifications_tx,
                    event_wakeup_tx.as_ref(),
                );
                break;
            }

            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }

            let event = match control_state.on_line(&line) {
                Ok(event) => event,
                Err(error) => {
                    fail_with_exit(
                        &mut current_command,
                        &pending_rx,
                        &notifications_tx,
                        &fatal_exit_tx,
                        &mut notifications,
                        event_wakeup_tx.as_ref(),
                        error,
                    );
                    break;
                }
            };

            match event {
                ControlStateEvent::None => {}
                ControlStateEvent::Notification(notification) => {
                    notifications.push(notification);
                }
                ControlStateEvent::CommandBegin => {
                    if current_command.is_some() {
                        // Requests can be composed as `cmd1 ; cmd2`, and tmux may emit
                        // a separate begin/end block for each sub-command. Keep the same
                        // pending request active until its completion token is observed.
                        continue;
                    }

                    match claim_pending_for_command_begin(&pending_rx) {
                        Ok(Some(pending)) => {
                            current_command =
                                Some(ActiveControlCommand::Tracked(TrackedPendingCommand {
                                    pending,
                                    output: String::new(),
                                }));
                        }
                        Ok(None) => {
                            // tmux may emit unsolicited startup/control blocks before the
                            // first app-issued command is tracked; consume these blocks
                            // without binding a pending request.
                            current_command = Some(ActiveControlCommand::Untracked);
                        }
                        Err(error) => {
                            fail_with_exit(
                                &mut current_command,
                                &pending_rx,
                                &notifications_tx,
                                &fatal_exit_tx,
                                &mut notifications,
                                event_wakeup_tx.as_ref(),
                                error,
                            );
                            break;
                        }
                    }
                }
                ControlStateEvent::CommandComplete { is_error, output } => {
                    let Some(active_command) = current_command.take() else {
                        fail_with_exit(
                            &mut current_command,
                            &pending_rx,
                            &notifications_tx,
                            &fatal_exit_tx,
                            &mut notifications,
                            event_wakeup_tx.as_ref(),
                            TmuxControlError::protocol(
                                "received command completion without a pending request",
                            ),
                        );
                        break;
                    };

                    let ActiveControlCommand::Tracked(mut tracked) = active_command else {
                        continue;
                    };

                    if is_error {
                        // tmux abandons the rest of a `;` command list after a
                        // failing command, so the completion token never arrives.
                        // Fail the request now instead of wedging the writer.
                        let response =
                            map_command_completion_response(&tracked.pending.command, true, output);
                        complete_pending_command(tracked.pending, response);
                        continue;
                    }

                    let Some(output_chunk) = split_control_completion_token(
                        output.as_str(),
                        tracked.pending.completion_token.as_str(),
                    ) else {
                        if !is_control_completion_token(output.as_str()) {
                            append_command_output_chunk(&mut tracked.output, output.as_str());
                        }
                        current_command = Some(ActiveControlCommand::Tracked(tracked));
                        continue;
                    };

                    append_command_output_chunk(&mut tracked.output, output_chunk.as_str());
                    let response = map_command_completion_response(
                        &tracked.pending.command,
                        false,
                        tracked.output,
                    );
                    complete_pending_command(tracked.pending, response);
                }
                ControlStateEvent::Exit(reason) => {
                    fail_all_pending(
                        &mut current_command,
                        &pending_rx,
                        TmuxControlError::channel(
                            reason
                                .clone()
                                .unwrap_or_else(|| "tmux control mode exited".to_string()),
                        ),
                    );
                    signal_fatal_exit(&fatal_exit_tx, reason.clone());
                    notifications.push(TmuxNotification::Exit(reason));
                    let _ = flush_notification_coalescer(
                        &mut notifications,
                        &notifications_tx,
                        event_wakeup_tx.as_ref(),
                    );
                    break;
                }
            }

            if let Err(error) = flush_notification_coalescer(
                &mut notifications,
                &notifications_tx,
                event_wakeup_tx.as_ref(),
            ) {
                fail_with_exit(
                    &mut current_command,
                    &pending_rx,
                    &notifications_tx,
                    &fatal_exit_tx,
                    &mut notifications,
                    event_wakeup_tx.as_ref(),
                    error,
                );
                break;
            }
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::channel::{ControlCommandResult, ControlRequest};
    use super::spawn_control_threads;
    use crate::tmux_control_core::types::{TmuxControlError, TmuxControlErrorKind};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    type Response = Result<ControlCommandResult, TmuxControlError>;

    /// Drives the worker threads against a scripted fake tmux.
    struct FakeTmux {
        request_tx: flume::Sender<ControlRequest>,
        commands: BufReader<UnixStream>,
        output: UnixStream,
    }

    impl FakeTmux {
        fn spawn() -> Self {
            let (worker_stdin, fake_stdin) = UnixStream::pair().expect("stdin pair");
            let (worker_stdout, fake_stdout) = UnixStream::pair().expect("stdout pair");
            fake_stdin
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read timeout");
            let (request_tx, request_rx) = flume::bounded(16);
            let (pending_tx, pending_rx) = flume::bounded(1);
            let (notifications_tx, _notifications_rx) = flume::bounded(64);
            let (fatal_exit_tx, _fatal_exit_rx) = flume::bounded(1);
            spawn_control_threads(
                None,
                worker_stdin,
                worker_stdout,
                request_rx,
                pending_tx,
                pending_rx,
                notifications_tx,
                fatal_exit_tx,
                None,
            );
            Self {
                request_tx,
                commands: BufReader::new(fake_stdin),
                output: fake_stdout,
            }
        }

        /// Enqueues `command` and returns the completion token the worker wrote.
        fn send(&mut self, command: &str, token: &str) -> flume::Receiver<Response> {
            let (response_tx, response_rx) = flume::bounded(1);
            self.request_tx
                .send(ControlRequest {
                    command: command.to_string(),
                    completion_token: token.to_string(),
                    response_tx: Some(response_tx),
                })
                .expect("enqueue");
            let mut line = String::new();
            self.commands
                .read_line(&mut line)
                .expect("worker should write the command to tmux");
            assert!(
                line.starts_with(command),
                "unexpected command line {line:?}"
            );
            response_rx
        }

        fn emit(&mut self, lines: &str) {
            self.output.write_all(lines.as_bytes()).expect("emit");
        }
    }

    fn recv(response_rx: &flume::Receiver<Response>) -> Response {
        response_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("command should complete")
    }

    #[test]
    fn error_block_fails_command_and_keeps_channel_usable() {
        let mut tmux = FakeTmux::spawn();

        // tmux 3.4 output for `select-pane -t %999 ; display-message -p <token>`:
        // the list stops at the error, so the token block never arrives.
        let failed = tmux.send("select-pane -t %999", "__termy_cmd_done_101");
        tmux.emit("%begin 1 10 1\ncan't find pane: %999\n%error 1 10 1\n");
        let error = recv(&failed).expect_err("failing command must fail");
        assert_eq!(error.kind, TmuxControlErrorKind::Runtime);
        assert_eq!(error.message, "can't find pane: %999");

        let ok = tmux.send("display-message -p ok", "__termy_cmd_done_102");
        tmux.emit(
            "%begin 1 11 1\nok\n%end 1 11 1\n%begin 1 12 1\n__termy_cmd_done_102\n%end 1 12 1\n",
        );
        assert_eq!(recv(&ok).expect("command succeeds").output.trim_end(), "ok");
    }

    #[test]
    fn stray_token_after_nested_error_is_not_next_command_output() {
        let mut tmux = FakeTmux::spawn();

        // A failing command nested in `if-shell` doesn't stop the outer list,
        // so its completion token still arrives after the error block.
        let failed = tmux.send(
            "if-shell true 'select-pane -t %999'",
            "__termy_cmd_done_201",
        );
        tmux.emit(
            "%begin 1 20 1\n%end 1 20 1\n%begin 1 21 1\ncan't find pane: %999\n%error 1 21 1\n",
        );
        assert!(recv(&failed).is_err());

        let next = tmux.send("display-message -p next", "__termy_cmd_done_202");
        tmux.emit(concat!(
            "%begin 1 22 1\n__termy_cmd_done_201\n%end 1 22 1\n",
            "%begin 1 23 1\nnext\n%end 1 23 1\n",
            "%begin 1 24 1\n__termy_cmd_done_202\n%end 1 24 1\n",
        ));
        assert_eq!(
            recv(&next).expect("command succeeds").output.trim_end(),
            "next"
        );
    }
}
