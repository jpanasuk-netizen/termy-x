internal import CTermy

/// The system appearance used to resolve light/dark themes.
public enum Appearance: UInt32, Hashable, Sendable {
    case light = 0
    case dark = 1
}

public enum ScrollbarVisibility: Hashable, Sendable {
    case off, always, onScroll

    init(rawValue: UInt32) {
        self = switch rawValue {
        case 1: .always
        case 2: .onScroll
        default: .off
        }
    }
}

public enum ScrollbarStyle: Hashable, Sendable {
    case neutral, mutedTheme, theme

    init(rawValue: UInt32) {
        self = switch rawValue {
        case 1: .mutedTheme
        case 2: .theme
        default: .neutral
        }
    }
}

public enum PaneFocusEffect: Hashable, Sendable {
    case off, softSpotlight, cinematic, minimal

    init(rawValue: UInt32) {
        self = switch rawValue {
        case 1: .softSpotlight
        case 2: .cinematic
        case 3: .minimal
        default: .off
        }
    }
}

/// Everything a host needs to draw a terminal the way the config describes it.
///
/// Cell metrics are measured by libtermy from the configured font. Use them
/// rather than guessing monospace ratios.
public struct RenderConfig: Hashable, Sendable {
    public var fontFamily: String
    public var activeTheme: String
    public var foreground: TermyColor
    public var background: TermyColor
    public var cursorColor: TermyColor
    public var fontSize: Float
    public var lineHeight: Float
    public var paddingX: Float
    public var paddingY: Float
    public var backgroundOpacity: Float
    /// Apply `backgroundOpacity` to explicit cell backgrounds too.
    public var backgroundOpacityAffectsCells: Bool
    public var backgroundBlur: Bool
    public var cursorBlink: Bool
    public var cursorStyle: CursorStyle
    public var cellWidth: Float
    public var cellHeight: Float
    public var mouseScrollMultiplier: Float
    public var scrollbarVisibility: ScrollbarVisibility
    public var scrollbarStyle: ScrollbarStyle
    public var copyOnSelect: Bool
    public var copyOnSelectToast: Bool
    public var paneFocusEffect: PaneFocusEffect
    public var paneFocusStrength: Float
    public var chromeContrast: Bool

    /// Metrics for `FrameStore.glyphPlan`.
    public var glyphMetrics: GlyphMetrics {
        GlyphMetrics(cellWidth: cellWidth, cellHeight: cellHeight, fontSize: fontSize)
    }

    init(_ config: TermyFfiRenderConfig) {
        fontFamily = config.font_family.copyString()
        activeTheme = config.active_theme.copyString()
        foreground = TermyColor(config.foreground)
        background = TermyColor(config.background)
        cursorColor = TermyColor(config.cursor)
        fontSize = config.font_size
        lineHeight = config.line_height
        paddingX = config.padding_x
        paddingY = config.padding_y
        backgroundOpacity = config.background_opacity
        backgroundOpacityAffectsCells = config.background_opacity_cells
        backgroundBlur = config.background_blur
        cursorBlink = config.cursor_blink
        cursorStyle = CursorStyle(rawValue: config.cursor_style)
        cellWidth = config.cell_width
        cellHeight = config.cell_height
        mouseScrollMultiplier = config.mouse_scroll_multiplier
        scrollbarVisibility = ScrollbarVisibility(rawValue: config.scrollbar_visibility)
        scrollbarStyle = ScrollbarStyle(rawValue: config.scrollbar_style)
        copyOnSelect = config.copy_on_select
        copyOnSelectToast = config.copy_on_select_toast
        paneFocusEffect = PaneFocusEffect(rawValue: config.pane_focus_effect)
        paneFocusStrength = config.pane_focus_strength
        chromeContrast = config.chrome_contrast
    }
}
