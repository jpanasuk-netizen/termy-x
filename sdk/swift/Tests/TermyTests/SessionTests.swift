import Foundation
import Testing
@testable import Termy

@Suite("Sessions")
@MainActor
struct SessionTests {
    @Test func displaySessionTracksStateAndFrames() throws {
        let session = try TerminalSession.display(size: TerminalSize(cols: 40, rows: 10))
        var changes: [FrameChange] = []
        session.onFrameChange = { changes.append($0) }

        try session.feed(Data("\u{1b}]2;Logs\u{07}\u{1b}]9;4;1;10\u{07}tail -f\r\n".utf8))
        #expect(session.title == "Logs")
        #expect(session.progress == .inProgress(10))
        #expect(session.frame.text(row: 0) == "tail -f")
        #expect(changes.count == 1)

        var replies = Data()
        session.onProtocolReply = { replies.append($0) }
        try session.feed(Data("\u{1b}[6n".utf8))
        #expect(String(decoding: replies, as: UTF8.self) == "\u{1b}[2;1R")
    }

    @Test func resizeForcesAFullFrame() throws {
        let session = try TerminalSession.display(size: TerminalSize(cols: 40, rows: 10))
        try session.feed(Data("x".utf8))
        var lastChange: FrameChange?
        session.onFrameChange = { lastChange = $0 }
        try session.resize(TerminalSize(cols: 60, rows: 12))
        #expect(lastChange?.damage == .full)
        #expect(session.frame.cols == 60)
    }

    /// End to end: a real PTY child process, the wakeup thread, and main-actor polling.
    @Test(.timeLimit(.minutes(1)))
    func ptySessionRunsACommand() async throws {
        let session = try TerminalSession(
            size: TerminalSize(cols: 60, rows: 10),
            options: TerminalOptions(
                startupCommand: "/bin/sh -c 'printf \"pty:%s\" \"$TERMY_SDK_TEST\"; sleep 0.2'",
                environment: ["TERMY_SDK_TEST": "ok"]
            )
        )
        defer { session.stop() }

        let deadline = Date().addingTimeInterval(10)
        while !session.frame.text(row: 0).contains("pty:ok"), Date() < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(session.frame.text(row: 0).contains("pty:ok"))
    }
}
