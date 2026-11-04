internal import CTermy

/// A resolved RGBA color.
public struct TermyColor: Hashable, Sendable {
    public var r: UInt8
    public var g: UInt8
    public var b: UInt8
    public var a: UInt8

    public init(r: UInt8, g: UInt8, b: UInt8, a: UInt8 = 255) {
        self.r = r
        self.g = g
        self.b = b
        self.a = a
    }

    /// Packs the color as `0xRRGGBBAA`, convenient as a cache key.
    public var rgba: UInt32 {
        UInt32(r) << 24 | UInt32(g) << 16 | UInt32(b) << 8 | UInt32(a)
    }

    init(_ color: TermyFfiColor) {
        self.init(r: color.r, g: color.g, b: color.b, a: color.a)
    }
}

/// One viewport cell.
///
/// Cells carry no position. Full frames are row-major and partial updates
/// follow their dirty spans in order.
public struct Cell: Hashable, Sendable {
    /// One Unicode scalar, not a complete grapheme cluster.
    public var codepoint: UInt32
    public var foreground: TermyColor
    public var background: TermyColor
    /// True when the cell uses the terminal's default background rather than
    /// an explicit color. Hosts with a translucent window leave these clear.
    public var usesDefaultBackground: Bool
    public var isBold: Bool
    public var isItalic: Bool
    public var isUnderlined: Bool
    public var isStrikethrough: Bool
    /// Whether to draw this cell's text. Backgrounds are painted regardless.
    public var rendersText: Bool
    /// The trailing column of a wide character; never draw a glyph here.
    public var isWideCharacterSpacer: Bool
    /// The row wraps onto the next row. Preserve this when copying text.
    public var isLineWrapped: Bool

    public var scalar: Unicode.Scalar? { Unicode.Scalar(codepoint) }

    /// The cell's printable text, or nil when nothing should be drawn.
    public var text: String? {
        guard rendersText, !isWideCharacterSpacer, codepoint > 0x20,
              let scalar else { return nil }
        return String(Character(scalar))
    }

    init(_ cell: TermyFfiCell) {
        codepoint = cell.codepoint
        foreground = TermyColor(cell.fg)
        background = TermyColor(cell.bg)
        usesDefaultBackground = cell.uses_terminal_default_bg
        isBold = cell.bold
        isItalic = cell.italic
        isUnderlined = cell.underline
        isStrikethrough = cell.strikethrough
        rendersText = cell.render_text
        isWideCharacterSpacer = cell.wide_character_spacer
        isLineWrapped = cell.line_wrapped
    }
}

/// The cursor shape libtermy reports.
public enum CursorStyle: Hashable, Sendable {
    case bar
    case block
    case unknown(UInt32)

    init(rawValue: UInt32) {
        self = switch rawValue {
        case 1: .bar
        case 2: .block
        default: .unknown(rawValue)
        }
    }
}

public struct TerminalCursor: Hashable, Sendable {
    public var isVisible: Bool
    public var row: Int
    public var col: Int
    public var style: CursorStyle

    public init(isVisible: Bool, row: Int, col: Int, style: CursorStyle) {
        self.isVisible = isVisible
        self.row = row
        self.col = col
        self.style = style
    }

    init(_ cursor: TermyFfiCursor) {
        self.init(
            isVisible: cursor.visible,
            row: cursor.row,
            col: cursor.col,
            style: CursorStyle(rawValue: cursor.style)
        )
    }
}

/// A changed run of cells on one viewport row. Both columns are inclusive.
public struct DirtySpan: Hashable, Sendable {
    public var row: Int
    public var leftCol: Int
    public var rightCol: Int

    public init(row: Int, leftCol: Int, rightCol: Int) {
        self.row = row
        self.leftCol = leftCol
        self.rightCol = rightCol
    }

    public var width: Int { rightCol - leftCol + 1 }

    init(_ span: TermyFfiDirtySpan) {
        self.init(row: span.row, leftCol: span.left_col, rightCol: span.right_col)
    }

    var ffi: TermyFfiDirtySpan {
        TermyFfiDirtySpan(row: row, left_col: leftCol, right_col: rightCol)
    }
}
