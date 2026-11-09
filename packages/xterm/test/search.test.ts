import { describe, expect, it } from 'vitest'
import { SearchAddon, cellSpans } from '../src/addons.ts'
import type { Terminal } from '../src/terminal.ts'
import type { IBufferCell, IBufferLine } from '../src/types.ts'

/** A line of cells as the engine stores them: wide characters are followed by a spacer. */
function fakeLine(text: string, cols: number): IBufferLine {
  const cells: Array<{ chars: string; width: number }> = []
  for (const chars of text) {
    const wide = /\p{Script=Han}|\p{Extended_Pictographic}/u.test(chars)
    cells.push({ chars, width: wide ? 2 : 1 })
    if (wide) cells.push({ chars: '', width: 0 })
  }
  while (cells.length < cols) cells.push({ chars: '', width: 1 })
  return {
    isWrapped: false,
    length: cols,
    getCell: (x) => {
      const cell = cells[x]
      return cell && ({ getChars: () => cell.chars, getWidth: () => cell.width } as IBufferCell)
    },
    translateToString: (trimRight = false) => {
      const full = cells
        .filter((cell) => cell.width > 0)
        .map((cell) => cell.chars || ' ')
        .join('')
      return trimRight ? full.replace(/ +$/, '') : full
    },
  }
}

describe('SearchAddon', () => {
  it('maps string indices to cell columns', () => {
    const spans = cellSpans(fakeLine('中a😀b', 8))
    // 中 covers cells 0-1, a cell 2, 😀 (two UTF-16 units) cells 3-4, b cell 5.
    expect(spans.start.slice(0, 5)).toEqual([0, 2, 3, 3, 5])
    expect(spans.end.slice(0, 5)).toEqual([2, 3, 5, 5, 6])
  })

  it('selects matches after wide characters in the right cells', () => {
    const selections: Array<[number, number, number]> = []
    const line = fakeLine('中文 error 😀 error', 30)
    const terminal = {
      rows: 1,
      buffer: { active: { length: 1, viewportY: 0, getLine: () => line } },
      getSelectionPosition: () => undefined,
      select: (col: number, row: number, length: number) => selections.push([col, row, length]),
      clearSelection: () => {},
      scrollToLine: () => {},
    } as unknown as Terminal
    const addon = new SearchAddon()
    addon.activate(terminal)
    expect(addon.findNext('error')).toBe(true)
    // "中文 " takes five cells, so the first match starts at column 5.
    expect(selections).toEqual([[5, 0, 5]])
    expect(addon.findPrevious('😀 e', { regex: false })).toBe(true)
    expect(selections[1]).toEqual([11, 0, 4])
  })
})
