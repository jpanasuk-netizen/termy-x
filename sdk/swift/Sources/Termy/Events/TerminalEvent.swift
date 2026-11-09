import Foundation
internal import CTermy

/// Terminal progress reported with OSC 9;4. Values are percentages.
public enum TerminalProgress: Hashable, Sendable {
    case clear
    case inProgress(UInt8)
    case error(UInt8)
    case indeterminate
    case warning(UInt8)

    init(state: UInt8, value: UInt8) {
        self = switch state {
        case 1: .inProgress(value)
        case 2: .error(value)
        case 3: .indeterminate
        case 4: .warning(value)
        default: .clear
        }
    }
}

/// Something that happened in the terminal besides cell changes.
public enum TerminalEvent: Hashable, Sendable {
    /// New work is pending; poll the terminal.
    case wakeup
    case title(String)
    /// Restore the host's fallback title.
    case resetTitle
    case bell
    /// The child process exited (PTY terminals).
    case exit
    /// A program wrote text to the clipboard with OSC 52. Apply your policy.
    case clipboardStore(String)
    case shellPromptStart
    case shellCommandStart
    case shellCommandExecuting
    /// `exitCode` is nil when the shell did not report one.
    case shellCommandFinished(exitCode: Int32?)
    case progress(TerminalProgress)
    case workingDirectory(String)
    /// The complete OSC 7501 snapshot. Replace, don't append.
    case programStatus([ProgramStatusRecord])
    /// Keys or a paste were discarded because the program's input backlog is full.
    case inputDropped
    /// An event kind newer than this SDK.
    case unknown(kind: UInt32)

    init(_ event: TermyFfiEvent) {
        switch event.kind {
        case TERMY_FFI_EVENT_WAKEUP.rawValue: self = .wakeup
        case TERMY_FFI_EVENT_TITLE.rawValue: self = .title(event.payload.copyString())
        case TERMY_FFI_EVENT_RESET_TITLE.rawValue: self = .resetTitle
        case TERMY_FFI_EVENT_BELL.rawValue: self = .bell
        case TERMY_FFI_EVENT_EXIT.rawValue: self = .exit
        case TERMY_FFI_EVENT_CLIPBOARD_STORE.rawValue:
            self = .clipboardStore(event.payload.copyString())
        case TERMY_FFI_EVENT_SHELL_PROMPT_START.rawValue: self = .shellPromptStart
        case TERMY_FFI_EVENT_SHELL_COMMAND_START.rawValue: self = .shellCommandStart
        case TERMY_FFI_EVENT_SHELL_COMMAND_EXECUTING.rawValue: self = .shellCommandExecuting
        case TERMY_FFI_EVENT_SHELL_COMMAND_FINISHED.rawValue:
            self = .shellCommandFinished(exitCode: event.exit_code == -1 ? nil : event.exit_code)
        case TERMY_FFI_EVENT_PROGRESS.rawValue:
            self = .progress(TerminalProgress(state: event.progress_state, value: event.progress_value))
        case TERMY_FFI_EVENT_WORKING_DIRECTORY.rawValue:
            self = .workingDirectory(event.payload.copyString())
        case TERMY_FFI_EVENT_PROGRAM_STATUS.rawValue:
            self = .programStatus(ProgramStatusRecord.decodeSnapshot(event.payload.copyData()))
        case TERMY_FFI_EVENT_INPUT_DROPPED.rawValue: self = .inputDropped
        default: self = .unknown(kind: event.kind)
        }
    }
}

/// One drained batch. When `hasMore` is true, drain again before waiting.
public struct TerminalEventBatch: Hashable, Sendable {
    public var events: [TerminalEvent]
    public var hasMore: Bool

    public init(events: [TerminalEvent], hasMore: Bool) {
        self.events = events
        self.hasMore = hasMore
    }

    init(_ batch: TermyFfiEventBatch) {
        events = UnsafeBufferPointer(start: batch.events_ptr, count: batch.events_len)
            .map(TerminalEvent.init)
        hasMore = batch.has_more
    }
}
