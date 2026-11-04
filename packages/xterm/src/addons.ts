import { Emitter, type IEvent } from '@termysh/web'
import type { Terminal } from './terminal.ts'
import type { IBufferLine, ITerminalAddon } from './types.ts'

/** Drop-in for `@xterm/addon-fit`. */
export class FitAddon implements ITerminalAddon {
  #terminal: Terminal | undefined

  activate(terminal: Terminal): void {
    this.#terminal = terminal
  }

  dispose(): void {
    this.#terminal = undefined
  }

  fit(): void {
    this.#terminal?.termy.fit()
  }

  proposeDimensions(): { cols: number; rows: number } | undefined {
    return this.#terminal?.termy.proposeDimensions()
  }
}

/** Drop-in for `@xterm/addon-web-links`: enables URL detection. */
export class WebLinksAddon implements ITerminalAddon {
  #handler: ((event: MouseEvent, uri: string) => void) | undefined
  #terminal: Terminal | undefined

  constructor(handler?: (event: MouseEvent, uri: string) => void, _options?: unknown) {
    this.#handler = handler
  }

  activate(terminal: Terminal): void {
    this.#terminal = terminal
    terminal.termy.setOptions({
      linkDetection: true,
      ...(this.#handler ? { linkHandler: { activate: this.#handler } } : {}),
    })
  }

  dispose(): void {
    this.#terminal?.termy.setOptions({ linkDetection: false })
    this.#terminal = undefined
  }
}

export interface ISearchOptions {
  regex?: boolean
  wholeWord?: boolean
  caseSensitive?: boolean
  incremental?: boolean
  decorations?: unknown
}

/** Drop-in for `@xterm/addon-search`. Matches are selected and scrolled into view. */
export class SearchAddon implements ITerminalAddon {
  #terminal: Terminal | undefined
  #onDidChangeResults = new Emitter<{ resultIndex: number; resultCount: number }>()
  readonly onDidChangeResults: IEvent<{ resultIndex: number; resultCount: number }> = this.#onDidChangeResults.event

  activate(terminal: Terminal): void {
    this.#terminal = terminal
  }

  dispose(): void {
    this.#onDidChangeResults.dispose()
    this.#terminal = undefined
  }

  findNext(term: string, options: ISearchOptions = {}): boolean {
    return this.#find(term, options, 1)
  }

  findPrevious(term: string, options: ISearchOptions = {}): boolean {
    return this.#find(term, options, -1)
  }

  clearDecorations(): void {}

  clearActiveDecoration(): void {
    this.#terminal?.clearSelection()
  }

  #find(term: string, options: ISearchOptions, direction: 1 | -1): boolean {
    const terminal = this.#terminal
    if (!terminal || term === '') return false
    const pattern = searchPattern(term, options)
    if (!pattern) return false
    const buffer = terminal.buffer.active
    // Matches are in cells: wide characters take two, and surrogate pairs and
    // combining marks take more than one string index.
    const matches: Array<{ line: number; col: number; length: number }> = []
    for (let line = 0; line < buffer.length; line++) {
      const bufferLine = buffer.getLine(line)
      const text = bufferLine?.translateToString(true) ?? ''
      let cells: CellSpans | undefined
      pattern.lastIndex = 0
      for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
        if (match[0].length === 0) {
          pattern.lastIndex++
          continue
        }
        cells ??= cellSpans(bufferLine!)
        const col = cells.start[match.index] ?? match.index
        const end = cells.end[match.index + match[0].length - 1] ?? col + match[0].length
        matches.push({ line, col, length: end - col })
      }
    }
    if (matches.length === 0) {
      terminal.clearSelection()
      this.#onDidChangeResults.fire({ resultIndex: -1, resultCount: 0 })
      return false
    }

    const current = terminal.getSelectionPosition()
    const origin = current
      ? direction === 1 && !options.incremental
        ? { line: current.start.y, col: current.start.x + 1 }
        : { line: current.start.y, col: current.start.x - (direction === -1 ? 1 : 0) }
      : { line: buffer.viewportY, col: 0 }
    const after = (m: { line: number; col: number }): boolean =>
      m.line > origin.line || (m.line === origin.line && m.col >= origin.col)
    let index =
      direction === 1
        ? matches.findIndex(after)
        : matches.findLastIndex((m) => !after(m) || (m.line === origin.line && m.col <= origin.col))
    if (index < 0) index = direction === 1 ? 0 : matches.length - 1
    const match = matches[index]!
    terminal.select(match.col, match.line, match.length)
    if (match.line < buffer.viewportY || match.line >= buffer.viewportY + terminal.rows) {
      terminal.scrollToLine(Math.max(0, match.line - Math.floor(terminal.rows / 2)))
    }
    this.#onDidChangeResults.fire({ resultIndex: index, resultCount: matches.length })
    return true
  }
}

/** For each string index of a line's text, the cells its character covers. */
export interface CellSpans {
  start: number[]
  end: number[]
}

/** Maps `translateToString` indices of `line` to cell columns. */
export function cellSpans(line: IBufferLine): CellSpans {
  const spans: CellSpans = { start: [], end: [] }
  for (let x = 0; x < line.length; x++) {
    const cell = line.getCell(x)
    const width = cell?.getWidth() ?? 1
    // Spacers after wide characters have no text of their own.
    if (width === 0) continue
    const chars = cell?.getChars() || ' '
    for (let unit = 0; unit < chars.length; unit++) {
      spans.start.push(x)
      spans.end.push(x + width)
    }
  }
  return spans
}

function searchPattern(term: string, options: ISearchOptions): RegExp | undefined {
  let source = options.regex ? term : term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  if (options.wholeWord) source = `\\b${source}\\b`
  try {
    return new RegExp(source, options.caseSensitive ? 'g' : 'gi')
  } catch {
    return undefined
  }
}

/**
 * Stand-in for renderer and Unicode addons (`@xterm/addon-webgl`,
 * `@xterm/addon-canvas`, `@xterm/addon-unicode11`, `@xterm/addon-image`).
 * Termy renders, measures widths and decodes kitty graphics natively.
 */
export class NoopAddon implements ITerminalAddon {
  activate(_terminal: Terminal): void {}
  dispose(): void {}
}

export class WebglAddon extends NoopAddon {
  readonly onContextLoss: IEvent<void> = new Emitter<void>().event
}
export class CanvasAddon extends NoopAddon {}
export class Unicode11Addon extends NoopAddon {}
