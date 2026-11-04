import AppKit
import Testing
import Termy
@testable import TermyAppKit

@Suite("Key mapping")
struct KeyMappingTests {
    @Test func namedKeys() {
        #expect(KeyMapping.namedKey(forKeyCode: 36) == "enter")
        #expect(KeyMapping.namedKey(forKeyCode: 126) == "up")
        #expect(KeyMapping.namedKey(forKeyCode: 122) == "f1")
        #expect(KeyMapping.namedKey(forKeyCode: 0) == nil)
    }

    @Test func printableKeysCarryTheirText() {
        let keystroke = KeyMapping.keystroke(
            keyCode: 0, characters: "A", charactersIgnoringModifiers: "A", modifierFlags: .shift)
        #expect(keystroke == Keystroke(key: "a", text: "A", modifiers: .shift))
    }

    @Test func controlCharactersAreNotText() {
        let keystroke = KeyMapping.keystroke(
            keyCode: 8, characters: "\u{03}", charactersIgnoringModifiers: "c", modifierFlags: .control)
        #expect(keystroke == Keystroke(key: "c", text: nil, modifiers: .control))
    }

    @Test func namedKeysIgnoreCharacters() {
        let keystroke = KeyMapping.keystroke(
            keyCode: 123, characters: "\u{F702}", charactersIgnoringModifiers: "\u{F702}",
            modifierFlags: [.option], isRepeat: true)
        #expect(keystroke == Keystroke(key: "left", modifiers: .alt, kind: .repeat))
    }

    @Test func modifierOnlyEventsProduceNothing() {
        #expect(KeyMapping.keystroke(
            keyCode: 56, characters: "", charactersIgnoringModifiers: "", modifierFlags: .shift) == nil)
    }

    @Test func mappedKeystrokesEncodeThroughLibtermy() throws {
        let terminal = try Terminal.display(size: TerminalSize(cols: 80, rows: 24))
        let keystroke = try #require(KeyMapping.keystroke(
            keyCode: 8, characters: "\u{03}", charactersIgnoringModifiers: "c", modifierFlags: .control))
        #expect(try terminal.encode(keystroke) == Data([0x03]))
    }
}
