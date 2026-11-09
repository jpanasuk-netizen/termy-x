internal import CTermy

/// A complete viewport, cells in row-major order.
public struct Frame: Hashable, Sendable {
    public var cols: Int
    public var rows: Int
    public var cells: [Cell]
    public var cursor: TerminalCursor
    /// Rows above the live bottom; zero means the viewport shows live output.
    public var displayOffset: Int
    /// Rows of retained scrollback history.
    public var historySize: Int

    public subscript(row: Int, col: Int) -> Cell {
        cells[row * cols + col]
    }

    /// The visible text of one row, without trailing blanks.
    public func text(row: Int) -> String {
        rowText(cells[row * cols ..< (row + 1) * cols])
    }

    public init(
        cols: Int, rows: Int, cells: [Cell], cursor: TerminalCursor, displayOffset: Int, historySize: Int
    ) {
        self.cols = cols
        self.rows = rows
        self.cells = cells
        self.cursor = cursor
        self.displayOffset = displayOffset
        self.historySize = historySize
    }

    init(_ frame: TermyFfiFrame) {
        self.init(
            cols: Int(frame.cols),
            rows: Int(frame.rows),
            cells: UnsafeBufferPointer(start: frame.cells_ptr, count: frame.cells_len).map(Cell.init),
            cursor: TerminalCursor(frame.cursor),
            displayOffset: frame.display_offset,
            historySize: frame.history_size
        )
    }
}

/// Which cells a frame update carries.
public enum FrameDamage: Hashable, Sendable {
    /// No cells changed; metadata such as the cursor may still have.
    case none
    /// Every cell, row-major.
    case full
    /// Cells for each span, concatenated in span order.
    case partial([DirtySpan])
}

/// Changed cells plus viewport metadata from `Terminal.takeFrameUpdate`.
public struct FrameUpdate: Hashable, Sendable {
    public var cols: Int
    public var rows: Int
    /// Full damage: `cols * rows` cells. Partial: cells for each span in order.
    public var cells: [Cell]
    public var damage: FrameDamage
    public var cursor: TerminalCursor
    public var displayOffset: Int
    public var historySize: Int

    public init(
        cols: Int, rows: Int, cells: [Cell], damage: FrameDamage, cursor: TerminalCursor,
        displayOffset: Int, historySize: Int
    ) {
        self.cols = cols
        self.rows = rows
        self.cells = cells
        self.damage = damage
        self.cursor = cursor
        self.displayOffset = displayOffset
        self.historySize = historySize
    }

    init(_ update: TermyFfiFrameUpdate) {
        self.init(
            cols: Int(update.cols),
            rows: Int(update.rows),
            cells: UnsafeBufferPointer(start: update.cells_ptr, count: update.cells_len).map(Cell.init),
            damage: FrameDamage(update),
            cursor: TerminalCursor(update.cursor),
            displayOffset: update.display_offset,
            historySize: update.history_size
        )
    }
}

extension FrameDamage {
    init(_ update: TermyFfiFrameUpdate) {
        switch update.damage_kind {
        case 1:
            self = .full
        case 2:
            let spans = UnsafeBufferPointer(start: update.spans_ptr, count: update.spans_len)
            self = .partial(spans.map(DirtySpan.init))
        default:
            self = .none
        }
    }
}

func rowText(_ cells: some Collection<Cell>) -> String {
    var text = ""
    var pendingSpaces = 0
    for cell in cells where !cell.isWideCharacterSpacer {
        if let scalar = cell.scalar, cell.rendersText, cell.codepoint > 0x20 {
            text.append(String(repeating: " ", count: pendingSpaces))
            pendingSpaces = 0
            text.unicodeScalars.append(scalar)
        } else {
            pendingSpaces += 1
        }
    }
    return text
}
