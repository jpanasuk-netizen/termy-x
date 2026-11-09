import Foundation
import Termy

/// A display terminal with output already fed.
func displayTerminal(cols: UInt16 = 80, rows: UInt16 = 24, feeding output: String = "") throws -> Terminal {
    let terminal = try Terminal.display(size: TerminalSize(cols: cols, rows: rows))
    if !output.isEmpty { try terminal.feed(output) }
    return terminal
}

/// Drains every pending event batch.
func drainAll(
    _ terminal: Terminal,
    clipboard: KittyClipboardHandler? = nil,
    onProtocolReply: ((Data) -> Void)? = nil
) throws -> [TerminalEvent] {
    var events: [TerminalEvent] = []
    var hasMore = true
    while hasMore {
        let batch = try terminal.drainEvents(clipboard: clipboard, onProtocolReply: onProtocolReply)
        events += batch.events
        hasMore = batch.hasMore
    }
    return events
}

extension String {
    var bytes: [UInt8] { Array(utf8) }
}
