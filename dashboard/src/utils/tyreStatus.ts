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

/** Surface temperatures a tyre is happy at, and where it came from. */
export interface TyreWindow {
  coldBelowC: number
  hotAboveC: number
  /** Beyond this the tyre is overheating, not just warm. */
  overheatAboveC: number
  /** "manual": set in the widget; "lmu": around the game's optimal
   *  temperature; "fixed": the widget's original thresholds. */
  source: 'manual' | 'lmu' | 'fixed'
}

/** The engineer's own thresholds, when set (°C, surface). */
export interface ManualTyreWindow { coldBelowC: number | null; hotAboveC: number | null }

/**
 * The window a tyre is judged against: the engineer's thresholds when set,
 * else around the compound's optimal temperature from the game — a wet tyre
 * and a hard slick are happy at very different temperatures — else fixed.
 */
export function tyreWindow(optimalC?: number | null, manual?: ManualTyreWindow | null): TyreWindow {
  const c = manual?.coldBelowC
  const h = manual?.hotAboveC
  if (c != null && h != null && h > c) {
    return { coldBelowC: c, hotAboveC: h, overheatAboveC: h + TEMP_HOT_BAND_C - TEMP_OK_BAND_C, source: 'manual' }
  }
  if (optimalC && optimalC > 0) {
    return { coldBelowC: optimalC - TEMP_OK_BAND_C, hotAboveC: optimalC + TEMP_OK_BAND_C, overheatAboveC: optimalC + TEMP_HOT_BAND_C, source: 'lmu' }
  }
  return { coldBelowC: FIXED_COLD_C, hotAboveC: FIXED_WARM_C, overheatAboveC: FIXED_HOT_C, source: 'fixed' }
}

/** Colour for a tyre surface temperature in its window. No colour without data. */
export function tyreTempColor(tempC: number, w: TyreWindow): string {
  // 0 or less: no reading (LMU sends 0 K, -273 °C, for a car it does not detail).
  if (tempC <= 0) return TEMP_COLORS.none
  if (tempC < w.coldBelowC) return TEMP_COLORS.cold
  if (tempC <= w.hotAboveC) return TEMP_COLORS.ok
  if (tempC <= w.overheatAboveC) return TEMP_COLORS.warm
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
export function tyreStatus(t: TireData, w: TyreWindow = tyreWindow(t.optimal_temp)): TyreStatus {
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
  const windowText = `${w.coldBelowC.toFixed(0)}–${w.hotAboveC.toFixed(0)} °C (${w.source === 'manual' ? 'set by hand' : w.source === 'lmu' ? `around LMU's optimal ${(t.optimal_temp ?? 0).toFixed(0)} °C` : 'default'})`
  if (surface > 0) {
    const ref = `Surface ${surface.toFixed(0)} °C, window ${windowText}.`
    if (surface > w.overheatAboveC) return { label: 'OVERHEAT', severity: 'danger', detail: ref }
    if (surface > w.hotAboveC) return { label: 'HOT', severity: 'warn', detail: ref }
    if (surface < w.coldBelowC) return { label: 'COLD', severity: 'info', detail: ref }
  }

  if (lockups > 0) {
    return { label: `LOCK ×${lockups}`, severity: 'info', detail: `${lockDetail} Too short to suggest a flat spot.` }
  }
  return { label: 'OK', severity: 'ok', detail: surface > 0 ? `In the window ${windowText}.` : 'Nothing to report.' }
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
