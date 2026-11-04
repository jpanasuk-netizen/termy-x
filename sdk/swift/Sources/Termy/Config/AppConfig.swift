internal import CTermy

public struct SafetyConfig: Hashable, Sendable {
    public var warnOnQuit: Bool
    public var warnOnQuitWithRunningProcess: Bool

    init(_ config: TermyFfiSafetyConfig) {
        warnOnQuit = config.warn_on_quit
        warnOnQuitWithRunningProcess = config.warn_on_quit_with_running_process
    }
}

/// Termy app settings exposed for native hosts. Enum-valued settings keep
/// libtermy's raw values; see the configuration reference for their meaning.
public struct NativeConfig: Hashable, Sendable {
    public var autoUpdate: Bool
    public var tmuxEnabled: Bool
    public var tmuxPersistence: Bool
    public var tmuxExclusive: Bool
    public var tmuxShowActivePaneBorder: Bool
    public var simpleMode: Bool
    public var nativeTabPersistence: Bool
    public var nativeLayoutAutosave: Bool
    public var nativeBufferPersistence: Bool
    public var showDebugOverlay: Bool
    public var onboardingComplete: Bool
    public var tabCloseVisibility: UInt32
    public var tabWidthMode: UInt32
    public var tabBarPosition: UInt32
    public var tabSwitchModifierHints: Bool
    public var chromeContrast: Bool
    public var commandPaletteShowKeybinds: Bool
    public var appIcon: UInt32
    public var shellIntegrationEnabled: Bool
    public var progressIndicatorEnabled: Bool
    public var autoHideTabBar: Bool
    public var showTermyInTitlebar: Bool
    public var macosOptionAsAlt: Bool

    init(_ config: TermyFfiNativeConfig) {
        autoUpdate = config.auto_update
        tmuxEnabled = config.tmux_enabled
        tmuxPersistence = config.tmux_persistence
        tmuxExclusive = config.tmux_exclusive
        tmuxShowActivePaneBorder = config.tmux_show_active_pane_border
        simpleMode = config.simple_mode
        nativeTabPersistence = config.native_tab_persistence
        nativeLayoutAutosave = config.native_layout_autosave
        nativeBufferPersistence = config.native_buffer_persistence
        showDebugOverlay = config.show_debug_overlay
        onboardingComplete = config.onboarding_complete
        tabCloseVisibility = config.tab_close_visibility
        tabWidthMode = config.tab_width_mode
        tabBarPosition = config.tab_bar_position
        tabSwitchModifierHints = config.tab_switch_modifier_hints
        chromeContrast = config.chrome_contrast
        commandPaletteShowKeybinds = config.command_palette_show_keybinds
        appIcon = config.app_icon
        shellIntegrationEnabled = config.shell_integration_enabled
        progressIndicatorEnabled = config.progress_indicator_enabled
        autoHideTabBar = config.auto_hide_tabbar
        showTermyInTitlebar = config.show_termy_in_titlebar
        macosOptionAsAlt = config.macos_option_as_alt
    }
}

public struct ConfigDiagnostic: Hashable, Sendable {
    public enum Kind: Hashable, Sendable {
        case unknownSection, unknownRootKey, unknownColorKey, invalidSyntax, invalidValue
        case duplicateRootKey, unknown(UInt32)

        init(rawValue: UInt32) {
            self = switch rawValue {
            case 1: .unknownSection
            case 2: .unknownRootKey
            case 3: .unknownColorKey
            case 4: .invalidSyntax
            case 5: .invalidValue
            case 6: .duplicateRootKey
            default: .unknown(rawValue)
            }
        }
    }

    public var lineNumber: Int
    public var kind: Kind
    public var message: String

    init(_ diagnostic: TermyFfiConfigDiagnostic) {
        lineNumber = diagnostic.line_number
        kind = Kind(rawValue: diagnostic.kind)
        message = diagnostic.message.copyString()
    }
}
