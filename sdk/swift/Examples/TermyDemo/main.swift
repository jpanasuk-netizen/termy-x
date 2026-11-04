import AppKit
import SwiftUI
import Termy
import TermyAppKit

/// A minimal terminal app: each window runs your configured shell.
///
///     scripts/build-libtermy.sh && swift run TermyDemo
struct TermyDemoApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup {
            TerminalWindow()
        }
        .defaultSize(width: 860, height: 540)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // `swift run` launches an unbundled executable; make it a regular app.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

@MainActor
final class TerminalModel: ObservableObject {
    let view: TerminalView?
    let error: String?
    @Published var title = "Termy"
    @Published var status: ProgramStatusRecord?

    init() {
        do {
            view = try TerminalView()
            error = nil
        } catch {
            view = nil
            self.error = "Could not start a terminal: \(error)"
        }
    }

    var background: Color {
        guard let color = view?.renderConfig.background else { return .black }
        return Color(red: Double(color.r) / 255, green: Double(color.g) / 255, blue: Double(color.b) / 255)
    }
}

struct TerminalWindow: View {
    @StateObject private var model = TerminalModel()

    var body: some View {
        Group {
            if let view = model.view {
                TermyTerminal(view: view)
                    .onTitleChange { model.title = $0 ?? "Termy" }
                    .onEvent { event in
                        if case .programStatus(let records) = event { model.status = records.mostUrgent }
                    }
                    .onExit { view.window?.close() }
            } else {
                Text(model.error ?? "").padding()
            }
        }
        .frame(minWidth: 360, minHeight: 200)
        .background(model.background)
        .navigationTitle(windowTitle)
    }

    private var windowTitle: String {
        guard let status = model.status else { return model.title }
        let badge = switch status.state {
        case .working: "●"
        case .blocked: "?"
        case .done: "✓"
        case .error: "!"
        case .idle: ""
        }
        return "\(badge) \(model.title)"
    }
}

TermyDemoApp.main()
