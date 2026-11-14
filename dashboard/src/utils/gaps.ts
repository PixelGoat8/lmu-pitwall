import type { VehicleScoring } from '../types/telemetry'

/** A gap between the player and another car. */
export type Gap =
  | { kind: 'time'; seconds: number }   // always >= 0: how far apart
  | { kind: 'laps'; laps: number }      // always >= 1
  | null                                 // not known yet

export interface ClassNeighbours {
  player: VehicleScoring
  ahead: VehicleScoring | null    // one place ahead in the player's class
  behind: VehicleScoring | null   // one place behind in the player's class
  classPosition: number           // 1-based
  classSize: number
}

/**
 * The cars one place ahead of and behind the player *within their class*.
 * In a multiclass field these are the rivals that matter; in a single-class
 * field they are simply the cars next in the order.
 */
export function classNeighbours(vehicles: VehicleScoring[], playerId: number): ClassNeighbours | null {
  const player = vehicles.find((v) => v.id === playerId)
  if (!player) return null
  const sameClass = vehicles
    .filter((v) => v.vehicle_class === player.vehicle_class)
    .sort((a, b) => a.position - b.position)
  const idx = sameClass.findIndex((v) => v.id === player.id)
  return {
    player,
    ahead: idx > 0 ? sameClass[idx - 1] : null,
    behind: idx >= 0 && idx < sameClass.length - 1 ? sameClass[idx + 1] : null,
    classPosition: idx + 1,
    classSize: sameClass.length,
  }
}

/**
 * How far `other` is from `player`.
 *
 * In a race this is the difference of the two cars' gaps to the leader, or a
 * whole number of laps when they are not on the same lap. In practice and
 * qualifying there is no order on track to measure, so it is the difference
 * of best laps. Same convention as the Proximity widget.
 */
export function gapBetween(player: VehicleScoring, other: VehicleScoring, isRace: boolean): Gap {
  if (isRace) {
    const lapDiff = Math.abs(other.laps_behind_leader - player.laps_behind_leader)
    if (lapDiff !== 0) return { kind: 'laps', laps: lapDiff }
    return { kind: 'time', seconds: Math.abs(other.time_behind_leader - player.time_behind_leader) }
  }
  if (other.best_lap_time <= 0 || player.best_lap_time <= 0) return null
  return { kind: 'time', seconds: Math.abs(other.best_lap_time - player.best_lap_time) }
}

export function fmtGap(gap: Gap): string {
  if (!gap) return '—'
  if (gap.kind === 'laps') return `${gap.laps} LAP${gap.laps > 1 ? 'S' : ''}`
  const s = gap.seconds
  if (s >= 60) {
    const m = Math.floor(s / 60)
    return `${m}:${(s - m * 60).toFixed(1).padStart(4, '0')}`
  }
  return s.toFixed(s < 10 ? 3 : 1)
}

/** One remembered gap, stamped with the session clock. */
export interface GapSample { t: number; seconds: number }

/**
 * Change of a gap over roughly one lap: negative = the gap shrank.
 *
 * Compared lap-over-lap rather than over a fixed few seconds because the
 * game refreshes a car's gap to the leader at timing points, not continuously:
 * over a short window the difference of two such values jumps around, while a
 * lap apart both cars have passed the same points.
 *
 * `null` until there is a sample about `windowS` old.
 */
export function gapTrend(history: GapSample[], now: number, windowS: number): number | null {
  if (history.length < 2) return null
  const target = now - windowS
  const latest = history[history.length - 1]
  // The newest sample that is at least one window old.
  let ref: GapSample | null = null
  for (const s of history) {
    if (s.t <= target) ref = s
    else break
  }
  if (!ref) return null
  // Too old to be "the previous lap" (e.g. after a long pit stop or a gap in data).
  if (now - ref.t > windowS * 1.6) return null
  return latest.seconds - ref.seconds
}
