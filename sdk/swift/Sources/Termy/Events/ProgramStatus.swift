import Foundation

/// The state a program reports with OSC 7501.
public enum ProgramState: String, Codable, Hashable, Sendable {
    case idle, working, done, blocked, error

    /// Higher values need the user's attention more urgently.
    public var urgency: Int {
        switch self {
        case .blocked: 4
        case .error: 3
        case .done: 2
        case .working: 1
        case .idle: 0
        }
    }
}

/// Why a blocked program is waiting.
public enum ProgramStatusKind: String, Codable, Hashable, Sendable {
    case permission, question, auth
}

/// One OSC 7501 record. Display strings are untrusted terminal content: render
/// them as plain text.
public struct ProgramStatusRecord: Codable, Hashable, Sendable {
    /// Empty for the root record. Slash-separated ids form a hierarchy.
    public var id: String
    public var state: ProgramState
    public var kind: ProgramStatusKind?
    /// Percentage, when reported.
    public var progress: UInt8?
    /// Resolved through ancestors when the record omits it.
    public var app: String?
    public var title: String?
    public var msg: String?

    public init(
        id: String, state: ProgramState, kind: ProgramStatusKind? = nil, progress: UInt8? = nil,
        app: String? = nil, title: String? = nil, msg: String? = nil
    ) {
        self.id = id
        self.state = state
        self.kind = kind
        self.progress = progress
        self.app = app
        self.title = title
        self.msg = msg
    }

    static func decodeSnapshot(_ data: Data) -> [ProgramStatusRecord] {
        (try? JSONDecoder().decode([ProgramStatusRecord].self, from: data)) ?? []
    }
}

extension Array where Element == ProgramStatusRecord {
    /// The most urgent non-idle record, as Termy's tab badge selects it.
    public var mostUrgent: ProgramStatusRecord? {
        filter { $0.state != .idle }.max { $0.state.urgency < $1.state.urgency }
    }
}
