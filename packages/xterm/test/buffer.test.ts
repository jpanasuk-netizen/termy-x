import type { Terminal as TermyTerminal } from '@termysh/web'
import { describe, expect, it } from 'vitest'
import { Buffer } from '../src/buffer.ts'

describe('Buffer', () => {
  it('reads only the active screen', () => {
    const term = {
      modes: { alternateScreen: false },
      cursor: { row: 1, col: 2 },
      historySize: 5,
      rows: 3,
      viewportY: 5,
    }
    const normal = new Buffer(term as unknown as TermyTerminal, 'normal')
    const alternate = new Buffer(term as unknown as TermyTerminal, 'alternate')
    expect([normal.length, normal.cursorX, normal.baseY]).toEqual([8, 2, 5])
    expect([alternate.length, alternate.cursorY, alternate.getLine(0)]).toEqual([0, 0, undefined])

    term.modes.alternateScreen = true
    expect(normal.length).toBe(0)
    expect(normal.getLine(0)).toBeUndefined()
    expect([alternate.length, alternate.cursorY]).toEqual([8, 1])
  })
})
