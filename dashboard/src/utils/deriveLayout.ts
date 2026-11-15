import type { LayoutItem } from 'react-grid-layout'

/**
 * A layout for a narrower screen, derived from the wide one.
 *
 * Left to itself the grid squeezes a 12-column layout into 10 or 6 columns
 * by clamping positions, which on a portrait tablet scatters widgets in a
 * staircase with holes. Instead, keep the wide layout's reading order (top
 * to bottom, left to right) and pack the widgets row by row at a width that
 * suits the narrower grid: in 6 columns small widgets go two to a row and
 * large ones take the full width.
 */
export function deriveLayout(wide: readonly LayoutItem[], cols: number): LayoutItem[] {
  const ordered = [...wide].sort((a, b) => a.y - b.y || a.x - b.x)
  const out: LayoutItem[] = []
  let x = 0
  let y = 0
  let rowH = 0
  for (const it of ordered) {
    const w = cols >= 10
      ? Math.min(cols, Math.max(2, Math.round((it.w * cols) / 12)))
      : it.w >= 5 ? cols : Math.ceil(cols / 2)
    if (x + w > cols) {
      y += rowH
      x = 0
      rowH = 0
    }
    out.push({ ...it, x, y, w, minW: Math.min(it.minW ?? 1, w) })
    x += w
    rowH = Math.max(rowH, it.h)
  }
  return out
}
