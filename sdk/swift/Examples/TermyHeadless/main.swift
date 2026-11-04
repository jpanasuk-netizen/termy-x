import Foundation
import Termy

/// Runs a command in a real PTY without a UI and prints the final screen.
///
///     swift run TermyHeadless              # parse sample output, no process
///     swift run TermyHeadless ls -la       # run a command in a PTY
@MainActor
func runHeadless() async throws {
    let command = CommandLine.arguments.dropFirst().joined(separator: " ")
    guard !command.isEmpty else {
        try parseSample()
        return
    }

    let session = try TerminalSession(
        size: TerminalSize(cols: 120, rows: 40),
        options: TerminalOptions(startupCommand: command)
    )
    session.onEvent = { event in
        if case .title(let title) = event { print("[title] \(title)") }
    }
    defer { session.stop() }

    let deadline = Date().addingTimeInterval(30)
    while !session.hasExited, Date() < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }
    session.poll()
    printScreen(session.frame)
    if !session.hasExited { print("[timed out after 30 s]") }
}

/// A display terminal: feed bytes from anywhere and read the parsed screen.
@MainActor
func parseSample() throws {
    let session = try TerminalSession.display(size: TerminalSize(cols: 60, rows: 6))
    session.onProtocolReply = { reply in print("[reply] \(reply.map { String(format: "%02x", $0) }.joined())") }
    try session.feed(Data((
        "\u{1b}]2;libtermy sample\u{07}"
            + "\u{1b}[1;32mbold green\u{1b}[0m and \u{1b}[4;38;2;122;162;247munderlined blue\u{1b}[0m\r\n"
            + "┌──────┐ wide: 日本語\r\n"
            + "└──────┘\u{1b}[6n"
    ).utf8))
    print("[title] \(session.title ?? "-")")
    printScreen(session.frame)
    let first = session.frame[0, 0]
    print("[cell 0,0] \(first.text ?? "") bold=\(first.isBold) fg=\(first.foreground)")
}

@MainActor
func printScreen(_ frame: FrameStore) {
    let rows = (0..<frame.rows).map(frame.text(row:))
    let lastUsed = rows.lastIndex { !$0.isEmpty } ?? -1
    rows.prefix(lastUsed + 1).forEach { print($0) }
}

try await runHeadless()
