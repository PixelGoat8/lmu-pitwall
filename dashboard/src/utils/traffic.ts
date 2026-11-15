import type { VehicleScoring } from '../types/telemetry'

/** A car closing on ours, or one ours is closing on, in another class. */
export interface TrafficCar {
  car: VehicleScoring
  /** "behind" = coming up to lap us; "ahead" = we are coming up to lap it. */
  side: 'behind' | 'ahead'
  /** Metres between the two cars along the track. */
  distance_m: number
  /** The same as seconds, at our pace. */
  gap_s: number
  /** Seconds until the faster car reaches the slower one at current pace; null if not closing. */
  catch_in_s: number | null
}

/** Only cars this close on track matter, in seconds of our pace. */
export const TRAFFIC_WINDOW_S = 20

/**
 * A car's representative lap: its last lap if it is a normal one, else its
 * best. In-laps, out-laps and laps behind a safety car would otherwise make
 * a Hypercar look slower than a GT3.
 */
export function paceOf(v: VehicleScoring): number | null {
  const best = v.best_lap_time > 0 ? v.best_lap_time : null
  const last = v.last_lap_time > 0 ? v.last_lap_time : null
  if (last && best && last <= best * 1.05) return last
  return best ?? last
}

/** Shortest signed distance along the track from `from` to `to`: positive = `to` is ahead. */
export function trackDelta(to: number, from: number, trackLength: number): number {
  let d = to - from
  if (trackLength > 0) {
    if (d > trackLength / 2) d -= trackLength
    else if (d < -trackLength / 2) d += trackLength
  }
  return d
}

/**
 * Cars of other classes near ours on track, with how soon the faster of the
 * two closes the gap at the pace each has shown.
 *
 * Only cars on the racing line count: LMU measures lap distance in the pit
 * lane from a different origin, so a car in the pits is nowhere meaningful.
 * Cars of our own class are left out: they are racing us, not traffic.
 */
export function traffic(vehicles: VehicleScoring[], playerId: number, trackLength: number): TrafficCar[] {
  const me = vehicles.find((v) => v.id === playerId)
  if (!me || trackLength <= 0 || me.in_pits || me.lap_dist < 0) return []
  const myPace = paceOf(me)
  if (!myPace) return []
  const myV = trackLength / myPace

  const out: TrafficCar[] = []
  for (const v of vehicles) {
    if (v.id === me.id || v.in_pits || v.lap_dist < 0 || v.vehicle_class === me.vehicle_class) continue
    const pace = paceOf(v)
    if (!pace) continue
    const d = trackDelta(v.lap_dist, me.lap_dist, trackLength)
    const gap = Math.abs(d) / myV
    if (gap > TRAFFIC_WINDOW_S) continue
    const theirV = trackLength / pace
    const side: TrafficCar['side'] = d < 0 ? 'behind' : 'ahead'
    // Behind: they close if faster. Ahead: we close if faster.
    const closing = side === 'behind' ? theirV - myV : myV - theirV
    out.push({
      car: v,
      side,
      distance_m: Math.abs(d),
      gap_s: gap,
      catch_in_s: closing > 0.05 ? Math.abs(d) / closing : null,
    })
  }
  // Soonest first; cars not closing at the end.
  return out
    .filter((t) => t.catch_in_s !== null)
    .sort((a, b) => (a.catch_in_s as number) - (b.catch_in_s as number))
}
