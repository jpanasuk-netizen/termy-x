// PTY master readiness and window-size helpers for the writer thread, and the
// wake/shutdown socket helpers. Included into `unix.rs`, so it shares that
// module's imports and items.

/// A socket pair for wake/shutdown bytes. Either end may outlive its peer, so
/// writes must not raise SIGPIPE: C and Swift hosts don't ignore it the way
/// Rust binaries do. Apple has no `MSG_NOSIGNAL`, so the socket opts out.
fn signal_socket_pair() -> io::Result<(UnixStream, UnixStream)> {
    let (first, second) = UnixStream::pair()?;
    #[cfg(target_vendor = "apple")]
    for stream in [&first, &second] {
        let enabled: c_int = 1;
        // SAFETY: the descriptor is live and the option value is a c_int of
        // the length passed.
        let result = unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_NOSIGPIPE,
                (&raw const enabled).cast(),
                size_of::<c_int>() as libc::socklen_t,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((first, second))
}

/// Sends one wake byte. A missing reader, a full buffer or any other error is
/// ignored: the byte only interrupts a poll that may already be gone.
fn send_signal_byte(stream: &UnixStream) {
    #[cfg(target_vendor = "apple")]
    const FLAGS: c_int = 0;
    #[cfg(not(target_vendor = "apple"))]
    const FLAGS: c_int = libc::MSG_NOSIGNAL;
    loop {
        // SAFETY: the descriptor is live and the one-byte buffer outlives the call.
        let sent = unsafe { libc::send(stream.as_raw_fd(), [1u8].as_ptr().cast(), 1, FLAGS) };
        if sent >= 0 || io::Error::last_os_error().kind() != ErrorKind::Interrupted {
            return;
        }
    }
}

fn wait_for_pty_writable(master: &File, wake: &UnixStream) -> io::Result<()> {
    let mut descriptors = [
        PollFd {
            fd: master.as_raw_fd(),
            events: POLLOUT,
            revents: 0,
        },
        PollFd {
            fd: wake.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        },
    ];
    // std's channel and poll cannot be waited on together, so Resize, Close
    // and replies also write to `wake`. Readiness still wakes immediately when
    // the child drains input, avoiding a fixed delay between partial writes.
    // SAFETY: both descriptors are live and exactly two entries are passed.
    let ready = unsafe {
        poll(
            descriptors.as_mut_ptr(),
            2 as PollCount,
            WRITER_POLL_TIMEOUT_MS,
        )
    };
    if ready < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(error);
        }
    }
    if descriptors[1].revents & POLLIN != 0 {
        let mut drained = [0; 64];
        while matches!((&*wake).read(&mut drained), Ok(read) if read == drained.len()) {}
    }
    let descriptor = &descriptors[0];
    if descriptor.revents & POLLNVAL != 0 {
        return Err(io::Error::new(
            ErrorKind::BrokenPipe,
            "Termy native PTY writer descriptor is invalid",
        ));
    }
    // A hung-up master stays ready, so retrying the write would spin until
    // the reader notices the exit and closes the writer. Linux reports POLLERR
    // together with POLLOUT once the other end is gone, so POLLERR alone fails.
    let hung_up = descriptor.revents & POLLHUP != 0 && descriptor.revents & POLLOUT == 0;
    if hung_up || descriptor.revents & POLLERR != 0 {
        return Err(io::Error::new(
            ErrorKind::BrokenPipe,
            "Termy native PTY was hung up",
        ));
    }
    Ok(())
}

fn resize_pty_master(master: &File, window_size: WinSize) -> io::Result<()> {
    loop {
        // SAFETY: `master` owns a live PTY descriptor and `window_size` has the
        // platform winsize layout for the duration of the ioctl call.
        if unsafe { ioctl(master.as_raw_fd(), TIOCSWINSZ, &window_size) } >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
