import Termy

/// A linear text selection between two viewport cells, in reading order.
public struct TerminalSelection: Hashable, Sendable {
    public enum Granularity: Hashable, Sendable { case character, word, line }

    /// Where the gesture started.
    public var anchor: GridPosition
    /// Where the pointer is now.
    public var head: GridPosition
    public var granularity: Granularity

    public init(anchor: GridPosition, head: GridPosition? = nil, granularity: Granularity = .character) {
        self.anchor = anchor
        self.head = head ?? anchor
        self.granularity = granularity
    }

    public var start: GridPosition { min(anchor, head) }
    public var end: GridPosition { max(anchor, head) }

    public func contains(_ position: GridPosition) -> Bool {
        start <= position && position <= end
    }

    /// The rows the selection touches.
    public var rows: ClosedRange<Int> { start.row...end.row }

    /// Expands the endpoints to whole words or lines in `frame`.
    public func expanded(in frame: FrameStore) -> TerminalSelection {
        guard !frame.isEmpty else { return self }
        var result = self
        let (low, high) = (start, end)
        switch granularity {
        case .character:
            return self
        case .line:
            result.anchor = GridPosition(row: low.row, col: 0)
            result.head = GridPosition(row: high.row, col: frame.cols - 1)
        case .word:
            result.anchor = Self.wordBoundary(from: low, in: frame, step: -1)
            result.head = Self.wordBoundary(from: high, in: frame, step: 1)
        }
        return result
    }

    /// The selected text.
    public func text(in frame: FrameStore) -> String {
        let selection = expanded(in: frame)
        return frame.text(from: selection.start, to: selection.end)
    }

    private static func isWordCharacter(_ cell: Cell) -> Bool {
        guard let text = cell.text, let scalar = text.unicodeScalars.first else { return false }
        return scalar.properties.isAlphabetic || ("0"..."9").contains(text)
            || "_-./~:@".unicodeScalars.contains(scalar)
    }

    private static func wordBoundary(from position: GridPosition, in frame: FrameStore, step: Int) -> GridPosition {
        let row = min(max(position.row, 0), frame.rows - 1)
        var col = min(max(position.col, 0), frame.cols - 1)
        guard isWordCharacter(frame[row, col]) else { return GridPosition(row: row, col: col) }
        while (0..<frame.cols).contains(col + step), isWordCharacter(frame[row, col + step]) {
            col += step
        }
        return GridPosition(row: row, col: col)
    }
}
