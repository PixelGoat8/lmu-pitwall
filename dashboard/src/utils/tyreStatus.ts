import type { TireData } from '../types/telemetry'

export type TyreSeverity = 'ok' | 'info' | 'warn' | 'danger'

export interface TyreStatus {
  /** Short label for the chip, e.g. "PUNCTURE", "FLAT SPOT?", "COLD". Empty when nothing to say. */
  label: string
  severity: TyreSeverity
  /** One sentence for the tooltip. */
  detail: string
}

// A single slide this long, or this much sliding in total, very likely left
// a flat spot. LMU does not publish flat spots, so this is an inference from
// how the tyre was braked, tuned to be quiet about ordinary small lock-ups.
const FLAT_SPOT_SINGLE_S = 0.5
const FLAT_SPOT_TOTAL_S = 1.5

// Around the compound's optimal temperature.
const TEMP_OK_BAND_C = 15
const TEMP_HOT_BAND_C = 30

// Fallback window when the game gives no optimal temperature (the widget's
// original fixed thresholds).
const FIXED_COLD_C = 60
const FIXED_WARM_C = 80
const FIXED_HOT_C = 100

const WEAR_LOW_PCT = 30
const WEAR_CRITICAL_PCT = 15

export const TEMP_COLORS = {
  none: '#333',
  cold: '#3b82f6',
  ok: '#22c55e',
  warm: '#facc15',
  hot: '#ef4444',
} as const

/**
 * Colour for a tyre temperature. Relative to the compound's optimal
 * temperature when the game provides it — a wet tyre and a hard slick are
 * happy at very different temperatures — and to fixed thresholds otherwise.
 */
export function tyreTempColor(tempC: number, optimalC?: number | null): string {
  if (tempC <= 0) return TEMP_COLORS.none
  if (optimalC && optimalC > 0) {
    const d = tempC - optimalC
    if (d < -TEMP_OK_BAND_C) return TEMP_COLORS.cold
    if (d <= TEMP_OK_BAND_C) return TEMP_COLORS.ok
    if (d <= TEMP_HOT_BAND_C) return TEMP_COLORS.warm
    return TEMP_COLORS.hot
  }
  if (tempC < FIXED_COLD_C) return TEMP_COLORS.cold
  if (tempC < FIXED_WARM_C) return TEMP_COLORS.ok
  if (tempC < FIXED_HOT_C) return TEMP_COLORS.warm
  return TEMP_COLORS.hot
}

/** True when a long or repeated lock-up makes a flat spot likely. */
export function flatSpotLikely(t: TireData): boolean {
  return (t.lockup_longest_s ?? 0) >= FLAT_SPOT_SINGLE_S || (t.lockup_total_s ?? 0) >= FLAT_SPOT_TOTAL_S
}

/**
 * The one thing most worth saying about a tyre, most serious first:
 * failures, then damage from lock-ups, then wear, then temperature.
 */
export function tyreStatus(t: TireData): TyreStatus {
  const none: TyreStatus = { label: '', severity: 'ok', detail: '' }
  const hasData = t.carcass_temp !== 0 || t.pressure !== 0 || t.brake_temp !== 0
  if (!hasData && !t.flat && !t.detached) return none

  if (t.detached) return { label: 'WHEEL OFF', severity: 'danger', detail: 'The wheel is detached.' }
  if (t.flat) return { label: 'PUNCTURE', severity: 'danger', detail: 'The tyre is flat.' }
  if (t.locked) return { label: 'LOCKED', severity: 'danger', detail: 'The wheel is locked right now.' }

  const wearPct = t.wear * 100
  const lockups = t.lockups ?? 0
  const lockDetail = lockups > 0
    ? `${lockups} lock-up${lockups > 1 ? 's' : ''} on this tyre, ${(t.lockup_total_s ?? 0).toFixed(1)} s in total, longest ${(t.lockup_longest_s ?? 0).toFixed(1)} s.`
    : ''

  if (flatSpotLikely(t)) {
    return {
      label: 'FLAT SPOT?',
      severity: 'warn',
      detail: `${lockDetail} A flat spot is likely (inferred from the lock-ups; the game does not report flat spots).`,
    }
  }
  if (wearPct > 0 && wearPct < WEAR_CRITICAL_PCT) {
    return { label: 'WORN OUT', severity: 'danger', detail: `Only ${wearPct.toFixed(0)}% tread left.` }
  }
  if (wearPct > 0 && wearPct < WEAR_LOW_PCT) {
    return { label: 'WORN', severity: 'warn', detail: `${wearPct.toFixed(0)}% tread left.` }
  }

  const surface = (t.temp_inner + t.temp_mid + t.temp_outer) / 3
  const opt = t.optimal_temp
  if (surface > 0 && opt && opt > 0) {
    const d = surface - opt
    const ref = `Surface ${surface.toFixed(0)} °C, optimal ${opt.toFixed(0)} °C.`
    if (d > TEMP_HOT_BAND_C) return { label: 'OVERHEAT', severity: 'danger', detail: ref }
    if (d > TEMP_OK_BAND_C) return { label: 'HOT', severity: 'warn', detail: ref }
    if (d < -TEMP_OK_BAND_C) return { label: 'COLD', severity: 'info', detail: ref }
  }

  if (lockups > 0) {
    return { label: `LOCK ×${lockups}`, severity: 'info', detail: `${lockDetail} Too short to suggest a flat spot.` }
  }
  return { label: 'OK', severity: 'ok', detail: opt ? `In the window around ${opt.toFixed(0)} °C.` : 'Nothing to report.' }
}

const POSITIONS = ['FL', 'FR', 'RL', 'RR'] as const

/**
 * A sentence when one tyre is clearly more worn than the rest, else `null`.
 * Ten points of tread is beyond what different corner loads explain on their
 * own and usually means a setup or driving issue worth mentioning.
 */
export function unevenWear(tires: TireData[]): string | null {
  const pct = tires.map((t) => t.wear * 100)
  if (pct.some((p) => p <= 0)) return null
  const min = Math.min(...pct)
  const others = pct.filter((p) => p !== min)
  if (!others.length) return null
  const restAvg = others.reduce((a, b) => a + b, 0) / others.length
  if (restAvg - min < 10) return null
  return `${POSITIONS[pct.indexOf(min)]} wearing fastest: ${min.toFixed(0)}% vs ${restAvg.toFixed(0)}%`
}
