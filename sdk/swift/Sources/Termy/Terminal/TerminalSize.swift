internal import CTermy

/// Grid dimensions plus the cell size in points.
///
/// The PTY reports pixel sizes from the cell size, so programs that draw
/// images can lay them out correctly. Take cell metrics from `RenderConfig`.
public struct TerminalSize: Hashable, Sendable {
    public var cols: UInt16
    public var rows: UInt16
    public var cellWidth: Float
    public var cellHeight: Float

    public init(cols: UInt16, rows: UInt16, cellWidth: Float, cellHeight: Float) {
        self.cols = cols
        self.rows = rows
        self.cellWidth = cellWidth
        self.cellHeight = cellHeight
    }

    /// libtermy's default size, with your grid dimensions.
    public init(cols: UInt16, rows: UInt16) {
        let defaults = termy_size_default()
        self.init(cols: cols, rows: rows, cellWidth: defaults.cell_width, cellHeight: defaults.cell_height)
    }

    public static var `default`: TerminalSize {
        let size = termy_size_default()
        return TerminalSize(cols: size.cols, rows: size.rows, cellWidth: size.cell_width, cellHeight: size.cell_height)
    }

    /// The largest grid that fits `width` x `height` points after padding.
    public static func fitting(
        width: Double, height: Double, renderConfig: RenderConfig
    ) -> TerminalSize {
        let cellWidth = Double(max(renderConfig.cellWidth, 1))
        let cellHeight = Double(max(renderConfig.cellHeight, 1))
        let usableWidth = width - 2 * Double(renderConfig.paddingX)
        let usableHeight = height - 2 * Double(renderConfig.paddingY)
        let cols = UInt16(clamping: max(1, Int((usableWidth / cellWidth).rounded(.down))))
        let rows = UInt16(clamping: max(1, Int((usableHeight / cellHeight).rounded(.down))))
        return TerminalSize(
            cols: cols, rows: rows, cellWidth: renderConfig.cellWidth, cellHeight: renderConfig.cellHeight)
    }

    var ffi: TermyFfiSize {
        TermyFfiSize(cols: cols, rows: rows, cell_width: cellWidth, cell_height: cellHeight)
    }
}

/// How to launch a PTY-backed terminal.
public struct TerminalOptions {
    /// Shell, font, scrollback and color settings. Nil uses built-in defaults.
    public var config: TermyConfig?
    public var workingDirectory: String?
    /// Run this command instead of the configured shell.
    public var startupCommand: String?
    /// Extra environment variables for the child process.
    public var environment: [String: String]

    public init(
        config: TermyConfig? = nil,
        workingDirectory: String? = nil,
        startupCommand: String? = nil,
        environment: [String: String] = [:]
    ) {
        self.config = config
        self.workingDirectory = workingDirectory
        self.startupCommand = startupCommand
        self.environment = environment
    }
}
