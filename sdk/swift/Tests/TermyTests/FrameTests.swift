import Testing
@testable import Termy

@Suite("Frames")
struct FrameTests {
    @Test func snapshotContainsFedText() throws {
        let terminal = try displayTerminal(feeding: "\u{1b}[32mhello\u{1b}[0m world\r\n")
        let frame = try terminal.snapshot()
        #expect(frame.cols == 80)
        #expect(frame.rows == 24)
        #expect(frame.cells.count == 80 * 24)
        #expect(frame.text(row: 0) == "hello world")
        #expect(frame.cursor.row == 1)
        #expect(frame.cursor.col == 0)
    }

    @Test func cellsCarryResolvedStyles() throws {
        let terminal = try displayTerminal(feeding: "\u{1b}[1;3;4;9;38;2;255;0;0;48;2;0;0;255mX\u{1b}[0mY")
        let frame = try terminal.snapshot()
        let styled = frame[0, 0]
        #expect(styled.text == "X")
        #expect(styled.isBold)
        #expect(styled.isItalic)
        #expect(styled.isUnderlined)
        #expect(styled.isStrikethrough)
        #expect(styled.foreground == TermyColor(r: 255, g: 0, b: 0))
        #expect(styled.background == TermyColor(r: 0, g: 0, b: 255))
        #expect(!styled.usesDefaultBackground)
        #expect(frame[0, 1].usesDefaultBackground)
    }

    @Test func wideCharactersUseASpacer() throws {
        let frame = try displayTerminal(feeding: "日本").snapshot()
        #expect(frame[0, 0].text == "日")
        #expect(frame[0, 1].isWideCharacterSpacer)
        #expect(frame[0, 2].text == "本")
        #expect(frame.text(row: 0) == "日本")
    }

    @Test func firstUpdateIsFullAndLaterUpdatesArePartial() throws {
        let terminal = try displayTerminal(feeding: "one")
        let full = try terminal.takeFrameUpdate(forceFull: true)
        #expect(full.damage == .full)
        #expect(full.cells.count == 80 * 24)

        try terminal.feed("\r\ntwo")
        let partial = try terminal.takeFrameUpdate()
        guard case .partial(let spans) = partial.damage else {
            Issue.record("expected partial damage, got \(partial.damage)")
            return
        }
        #expect(partial.cells.count == spans.reduce(0) { $0 + $1.width })
        #expect(spans.contains { $0.row == 1 })

        let idle = try terminal.takeFrameUpdate()
        #expect(idle.damage == .none)
        #expect(idle.cells.isEmpty)
    }

    @Test func frameStoreMatchesSnapshotAfterPartialUpdates() throws {
        let terminal = try displayTerminal(cols: 40, rows: 10)
        let store = FrameStore()
        try terminal.updateFrame(store)
        for line in 0..<25 {
            try terminal.feed("line \(line) \u{1b}[3\(line % 8)mcolor\u{1b}[0m\r\n")
            try terminal.updateFrame(store)
        }
        try terminal.feed("\u{1b}[5;5Hpatched")
        let change = try terminal.updateFrame(store)
        #expect(change.needsDisplay)
        #expect(store.frame == (try terminal.snapshot()))
        #expect(store.text(row: 4).contains("patched"))
    }

    @Test func frameStoreRejectsPartialUpdateBeforeFullFrame() throws {
        let store = FrameStore()
        let update = FrameUpdate(
            cols: 2, rows: 1, cells: [], damage: .partial([DirtySpan(row: 0, leftCol: 0, rightCol: 0)]),
            cursor: TerminalCursor(isVisible: true, row: 0, col: 0, style: .block),
            displayOffset: 0, historySize: 0)
        #expect(throws: FrameStoreError.needsFullFrame) { try store.apply(update) }
    }

    @Test func resizeChangesDimensions() throws {
        let terminal = try displayTerminal(feeding: "resize me")
        let store = FrameStore()
        try terminal.updateFrame(store)
        try terminal.resize(TerminalSize(cols: 100, rows: 30))
        let change = try terminal.updateFrame(store)
        #expect(change.resized)
        #expect(store.cols == 100)
        #expect(store.rows == 30)
        #expect(store.text(row: 0) == "resize me")
    }

    @Test func textAcrossWrappedRowsJoinsWithoutNewline() throws {
        let terminal = try displayTerminal(cols: 10, rows: 5, feeding: "0123456789abcde\r\nnext")
        let store = FrameStore()
        try terminal.updateFrame(store)
        let text = store.text(from: GridPosition(row: 0, col: 0), to: GridPosition(row: 2, col: 9))
        #expect(text == "0123456789abcde\nnext")
    }

    @Test func textOutsideTheGridIsEmpty() throws {
        // A selection made before a shrinking resize can lie past the last row.
        let terminal = try displayTerminal(cols: 10, rows: 5, feeding: "hello")
        let store = FrameStore()
        try terminal.updateFrame(store)
        #expect(store.text(from: GridPosition(row: 8, col: 0), to: GridPosition(row: 9, col: 3)).isEmpty)
        #expect(store.text(from: GridPosition(row: -3, col: 0), to: GridPosition(row: -1, col: 3)).isEmpty)
        #expect(store.text(from: GridPosition(row: 0, col: 0), to: GridPosition(row: 9, col: 9)).hasPrefix("hello"))
    }

    @Test func scrollbackMovesTheViewport() throws {
        let terminal = try displayTerminal(cols: 20, rows: 5)
        for line in 0..<30 { try terminal.feed("row \(line)\r\n") }
        #expect(try terminal.scroll(lines: 3))
        let scrolled = try terminal.snapshot()
        #expect(scrolled.displayOffset == 3)
        #expect(scrolled.historySize > 0)
        #expect(try terminal.scrollToBottom())
        #expect(try terminal.snapshot().displayOffset == 0)
        #expect(try terminal.clearScrollback())
        #expect(try terminal.snapshot().historySize == 0)
    }
}
