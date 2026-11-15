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
 * Race gap between two cars, in either order.
 *
 * Summed along the order from each car's gap to the car one place ahead
 * (mTimeBehindNext / mLapsBehindNext). The gap to the leader cannot be used:
 * LMU sets mTimeBehindLeader to 0 for every car the leader has lapped, so two
 * lapped cars on the same lap would read 0 s apart (seen live, 2026-11-15).
 * Same rule as bridge/src/gaps.rs.
 */
export function raceGap(vehicles: VehicleScoring[], a: VehicleScoring, b: VehicleScoring): Gap {
  const [front, back] = a.position <= b.position ? [a, b] : [b, a]
  const byPlace = new Map(vehicles.map((v) => [v.position, v]))
  let seconds = 0
  let laps = 0
  for (let p = front.position + 1; p <= back.position; p++) {
    const v = byPlace.get(p)
    if (!v) return null
    // An older bridge or recording without these fields: the old way, right
    // at least while both cars are on the lead lap.
    if (v.time_behind_next === undefined || v.laps_behind_next === undefined) return leaderGap(front, back)
    if (v.laps_behind_next > 0) laps += v.laps_behind_next
    // Slightly negative values happen at timing points.
    else seconds += Math.max(0, v.time_behind_next)
  }
  return laps > 0 ? { kind: 'laps', laps } : { kind: 'time', seconds }
}

function leaderGap(front: VehicleScoring, back: VehicleScoring): Gap {
  const laps = back.laps_behind_leader - front.laps_behind_leader
  if (laps > 0) return { kind: 'laps', laps }
  return { kind: 'time', seconds: Math.max(0, back.time_behind_leader - front.time_behind_leader) }
}

/**
 * How far `other` is from `player`.
 *
 * In a race: `raceGap`, in time or in whole laps. In practice and qualifying
 * there is no order on track to measure, so it is the difference of best laps.
 */
export function gapBetween(vehicles: VehicleScoring[], player: VehicleScoring, other: VehicleScoring, isRace: boolean): Gap {
  if (isRace) return raceGap(vehicles, player, other)
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
