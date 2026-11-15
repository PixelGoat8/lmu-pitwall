/**
 * Recording a track outline from cars' position samples, one lap per car.
 * Pure functions over a `Build`, so the Track Map widget only schedules them.
 */

import { OUTLINE_VERSION, calcBounds, type TrackOutline, type TrackPoint } from './trackOutline'
import type { VehicleScoring } from '../types/telemetry'

/** Points closer together than this (m) are dropped — 5 m is plenty of detail. */
export const MIN_POINT_SPACING = 5
/** A step longer than this (m) is either a gap in the data or a teleport. */
export const MAX_POINT_STEP = 50
/** A gap in the data up to this long (m of lap distance) is bridged with a
 *  straight line; a recorded lap with a longer hole is not kept. */
export const MAX_DATA_GAP = 120
/** Fewer points than this is not a lap worth keeping. */
export const MIN_LAP_POINTS = 50

export interface Build {
  trackName: string
  points: TrackPoint[]
  /** S/F crossings seen: 0 = outlap in progress, 1 = recording a lap. */
  crossings: number
  sector1Dist: number
  sector2Dist: number
  prevDist: number
  prevCurS1: number
  prevCurS2: number
  last: TrackPoint | null
  /** The car jumped (respawn, teleport): this lap is not usable. */
  broken: boolean
}

/**
 * Starting value for the previous sector times.
 *
 * The sector lines are found by watching a sector time go from "none" to a
 * value. Crossing S/F clears those times, but scoring only refreshes at 5 Hz,
 * so for a moment after the line the *previous* lap's times are still being
 * reported. Starting from a positive value means the watch cannot fire on that
 * stale reading — it arms itself only after seeing the times actually clear.
 */
const NOT_YET_CLEARED = Infinity

export function freshBuild(trackName: string): Build {
  const build = {
    trackName, points: [], crossings: 0,
    sector1Dist: -1, sector2Dist: -1,
    prevDist: 0, prevCurS1: NOT_YET_CLEARED, prevCurS2: NOT_YET_CLEARED, last: null,
    broken: false,
  }
  return build
}

/** Begin a recording lap. The car is at the line when this is called. */
export function restartLap(build: Build): void {
  build.points = []
  build.last = null
  build.crossings = 1
  build.sector1Dist = -1
  build.sector2Dist = -1
  build.prevCurS1 = NOT_YET_CLEARED
  build.prevCurS2 = NOT_YET_CLEARED
  build.broken = false
}

/** Back to waiting for the line (the car went into the pits). */
export function abandonLap(build: Build): void {
  restartLap(build)
  build.crossings = 0
}

/**
 * The lap just finished as an outline, if it is good enough to keep: enough
 * points, from the line round to the line, no hole longer than
 * `MAX_DATA_GAP`. A lap with a hole would draw a straight line across the
 * infield and then be kept for good.
 */
export function finishLap(build: Build, length: number): TrackOutline | null {
  const pts = build.points
  if (build.broken || pts.length < MIN_LAP_POINTS) return null
  if (pts[0].d > MAX_DATA_GAP || pts[pts.length - 1].d < length - MAX_DATA_GAP) return null
  for (let i = 1; i < pts.length; i++) {
    if (pts[i].d - pts[i - 1].d > MAX_DATA_GAP) return null
  }
  return {
    version: OUTLINE_VERSION,
    trackName: build.trackName,
    points: pts,
    bounds: calcBounds(pts),
    complete: true,
    sector1Dist: build.sector1Dist,
    sector2Dist: build.sector2Dist,
  }
}

/**
 * One position sample of one car into its lap. Returns a finished outline
 * when the car has just completed a usable lap.
 */
export function stepBuild(build: Build, car: VehicleScoring, length: number): TrackOutline | null {
  const dist = car.lap_dist
  const prev = build.prevDist
  build.prevDist = dist

  // S/F crossing: lap distance falls from near a full lap back to near zero.
  if (prev > length * 0.7 && dist < length * 0.1) {
    const done = build.crossings >= 1 ? finishLap(build, length) : null
    restartLap(build)
    return done
  }
  if (build.crossings < 1) return null

  // The pit lane is not the track. A lap that visits it is abandoned, and
  // the next S/F crossing starts a new attempt.
  if (car.in_pits) {
    abandonLap(build)
    return null
  }
  if (build.broken) return null

  // Sector lines, caught the moment the game starts reporting a time for
  // the sector just left. That lands within one scoring tick of the real
  // line, which at map scale is well under a pixel.
  if (build.sector1Dist < 0 && build.prevCurS1 <= 0 && car.cur_sector1 > 0) build.sector1Dist = dist
  if (build.sector2Dist < 0 && build.prevCurS2 <= 0 && car.cur_sector2 > 0) build.sector2Dist = dist
  build.prevCurS1 = car.cur_sector1
  build.prevCurS2 = car.cur_sector2

  const last = build.last
  if (last) {
    const step = Math.hypot(car.pos_x - last.x, car.pos_z - last.z)
    if (step < MIN_POINT_SPACING) return null
    if (step > MAX_POINT_STEP) {
      // A late update (a hub, a slow link) or a jump? Driving on, the lap
      // distance covered matches the straight-line step, give or take the
      // bends; after a respawn it does not.
      const along = dist - last.d
      const driving = along > 0 && along <= MAX_DATA_GAP && step <= along * 1.05 + 5 && step >= along * 0.5
      if (!driving) {
        build.broken = true
        return null
      }
    }
  }

  const pt: TrackPoint = { x: car.pos_x, y: car.pos_y, z: car.pos_z, d: dist }
  build.points.push(pt)
  build.last = pt
  return null
}

/** Record when nothing usable is stored, or only an older kind of record. */
export function needsRecording(o: TrackOutline | null): boolean {
  return !o || !o.complete || o.version < OUTLINE_VERSION
}

export function hasSectorLines(o: TrackOutline): boolean {
  return o.sector1Dist > 0 && o.sector2Dist > o.sector1Dist
}
