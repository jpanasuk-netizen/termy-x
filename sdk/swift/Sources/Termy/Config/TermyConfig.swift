import Foundation
internal import CTermy

/// A parsed Termy configuration.
///
/// Load the user's config with `loadDefault()`, a specific file with
/// `init(path:)`, or literal text with `init(contents:)`. Parse problems do not
/// throw; read them from `diagnostics()`.
public final class TermyConfig {
    let handle: OpaquePointer

    private init(handle: OpaquePointer) {
        self.handle = handle
    }

    deinit {
        termy_config_free(handle)
    }

    /// The user's config file, or defaults when none exists.
    public static func loadDefault() throws(TermyError) -> TermyConfig {
        var handle: OpaquePointer?
        try check(termy_config_load_default(&handle), "termy_config_load_default")
        return TermyConfig(handle: try require(handle, "termy_config_load_default"))
    }

    public convenience init(path: String) throws(TermyError) {
        var handle: OpaquePointer?
        let status = withUTF8(path) { termy_config_load_path($0, $1, &handle) }
        try check(status, "termy_config_load_path")
        self.init(handle: try Self.require(handle, "termy_config_load_path"))
    }

    public convenience init(contents: String) throws(TermyError) {
        var handle: OpaquePointer?
        let status = withUTF8(contents) { termy_config_from_contents($0, $1, &handle) }
        try check(status, "termy_config_from_contents")
        self.init(handle: try Self.require(handle, "termy_config_from_contents"))
    }

    /// True when the config came from a file rather than built-in defaults.
    public var isLoadedFromDisk: Bool { termy_config_loaded_from_disk(handle) }

    /// The config file path.
    public var path: String? { try? takeString("termy_config_path") { termy_config_path(handle, $0) } }

    public var scrollbackHistory: Int { termy_config_runtime_scrollback_history(handle) }

    /// The reduced scrollback for inactive tabs, when enabled.
    public var inactiveTabScrollback: Int? {
        var enabled = false
        var value = 0
        guard termy_config_runtime_inactive_tab_scrollback(handle, &enabled, &value) == TERMY_FFI_OK,
              enabled else { return nil }
        return value
    }

    /// The configured initial window size in points.
    public var windowSize: (width: Float, height: Float)? {
        var width: Float = 0
        var height: Float = 0
        guard termy_config_window_size(handle, &width, &height) == TERMY_FFI_OK else { return nil }
        return (width, height)
    }

    public var workingDirectory: String? {
        let value = try? takeString("termy_config_working_directory") {
            termy_config_working_directory(handle, $0)
        }
        return value?.isEmpty == false ? value : nil
    }

    public var tmuxBinary: String? {
        try? takeString("termy_config_tmux_binary") { termy_config_tmux_binary(handle, $0) }
    }

    public var uiFontFamily: String? {
        try? takeString("termy_config_ui_font_family") { termy_config_ui_font_family(handle, $0) }
    }

    public func safety() throws(TermyError) -> SafetyConfig {
        var safety = TermyFfiSafetyConfig()
        try check(termy_config_safety(handle, &safety), "termy_config_safety")
        return SafetyConfig(safety)
    }

    public func native() throws(TermyError) -> NativeConfig {
        var native = TermyFfiNativeConfig()
        try check(termy_config_native(handle, &native), "termy_config_native")
        return NativeConfig(native)
    }

    /// Render settings with themes resolved for `appearance`.
    public func renderConfig(for appearance: Appearance = .dark) throws(TermyError) -> RenderConfig {
        var config = TermyFfiRenderConfig()
        let status = termy_config_render_config_for_appearance(handle, appearance.rawValue, &config)
        defer { termy_render_config_free(&config) }
        try check(status, "termy_config_render_config_for_appearance")
        return RenderConfig(config)
    }

    public var diagnosticCount: Int { termy_config_diagnostic_count(handle) }

    public func diagnostics() throws(TermyError) -> [ConfigDiagnostic] {
        var batch = TermyFfiConfigDiagnosticBatch()
        let status = termy_config_diagnostics(handle, &batch)
        defer { termy_config_diagnostics_free(&batch) }
        try check(status, "termy_config_diagnostics")
        return UnsafeBufferPointer(start: batch.diagnostics_ptr, count: batch.diagnostics_len)
            .map(ConfigDiagnostic.init)
    }

    /// Configured tasks as JSON.
    public func tasksJSON() throws(TermyError) -> Data {
        try takeBytes("termy_config_tasks_json") { termy_config_tasks_json(handle, $0) }
    }

    /// Configured keybindings as JSON.
    public func keybindsJSON() throws(TermyError) -> Data {
        try takeBytes("termy_config_keybinds_json") { termy_config_keybinds_json(handle, $0) }
    }

    /// The settings schema with this config's current values, as JSON.
    public func settingsSchemaJSON() throws(TermyError) -> Data {
        try takeBytes("termy_settings_schema_json") { termy_settings_schema_json(handle, $0) }
    }

    private static func require(_ handle: OpaquePointer?, _ operation: String) throws(TermyError) -> OpaquePointer {
        guard let handle else { throw TermyError(code: .nullPointer, operation: operation) }
        return handle
    }
}
