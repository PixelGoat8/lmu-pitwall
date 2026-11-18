/**
 * Track Map — the field's live positions on a track outline recorded from a lap.
 *
 * Two things here are worth knowing before changing anything.
 *
 * **Cars do not move by re-rendering.** Position samples arrive in discrete
 * updates; a frame drawn straight from the newest one steps between them, which
 * on screen is a stutter. Instead every update is timestamped into a per-car
 * buffer, and a requestAnimationFrame loop writes an interpolated `transform`
 * onto each marker directly, at the display's refresh rate. React renders the
 * markers; it does not animate them, and it does not re-render when they move.
 *
 * **The outline is recorded once and kept.** See `utils/trackOutline.ts`. An
 * outline stored by an older version stays on screen and is quietly replaced by
 * a fuller record on the next clean lap — nothing has to be re-recorded by hand.
 * Every car on track is recorded at once, and the first clean lap of any of
 * them makes the map. The result is stored in the browser and on the bridge
 * (or hub), so every other dashboard gets it without a lap of its own.
 */

import { useRef, useMemo, useEffect, useState, useCallback } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { sendBridgeCommand } from '../../hooks/useWebSocket'
import { classNeighbours } from '../../utils/gaps'
import { useSettingsStore } from '../../stores/settingsStore'
import { colors, fonts } from '../../styles/theme'
import { CLASS_COLORS, getClassColor } from '../../utils/classColors'
import {
  SVG_SIZE, FALLBACK_BOUNDS,
  calcBounds, unionBounds, boundsEqual, worldToSvg, toPolyline, crossMark,
  splitSectors, indexAtDistance, computeStats, elevationPath, hasDetail,
  loadOutline, saveOutline, parseOutline, isBetterOutline,
  type Bounds, type TrackOutline, type TrackPoint,
} from '../../utils/trackOutline'
import { newTrail, pushSample, renderDelay, sampleAt, type Trail } from '../../utils/motionTrail'
import { type Build, freshBuild, stepBuild, needsRecording, hasSectorLines } from '../../utils/trackRecorder'

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PLAYER_STROKE = '#facc15'

/** Road colour per sector — three greys, distinct enough to read as three
 *  sections without competing with the class colours the cars are drawn in. */
const SECTOR_FILL = ['#3d3d3d', '#2d2d2d', '#1f1f1f']
const SECTOR_YELLOW = '#6b5410'
const TRACK_FILL = '#2d2d2d'

/** How often the outline recorder samples the player's position. */
const RECORD_INTERVAL_MS = 50
/** Refit the view and redraw the lap being recorded on this interval, rather
 *  than on every update — bounds that move at 20 Hz make the map shimmer. */
const REFIT_MS = 400

const ELEV_STRIP_H = 46

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

const km = (m: number) => (m / 1000).toFixed(3) + ' km'
const kmShort = (m: number) => (m / 1000).toFixed(2)
const metres = (m: number) => Math.round(m) + ' m'

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

interface Marker {
  id: number
  color: string
  label: number
  isPlayer: boolean
  /** The car one place ahead of or behind ours in our class. */
  isRival: boolean
}

/** Ring drawn around the class rivals directly ahead and behind. */
const RIVAL_STROKE = '#f97316'

export default function TrackMap() {
  const trackName = useTelemetryStore((s) => s.session.track_name)
  const trackLength = useTelemetryStore((s) => s.session.track_length)
  const detail = useSettingsStore((s) => s.trackMapDetail)

  // The cars on screen, with their class colour and rank within that class.
  //
  // The selector returns the previous array unchanged unless the field itself
  // changed, so React stays out of the 20 Hz update path entirely: a re-render
  // happens when a car joins, leaves or changes position, not when one moves.
  const rosterCache = useRef<{ key: string; list: Marker[] }>({ key: '', list: [] })
  const roster = useTelemetryStore((s) => {
    const { vehicles, player_vehicle_id } = s.scoring
    let key = String(player_vehicle_id)
    for (const v of vehicles) key += `|${v.id}:${v.vehicle_class}:${v.position}`
    if (key === rosterCache.current.key) return rosterCache.current.list

    const rankInClass = new Map<string, number>()
    const n = classNeighbours(vehicles, player_vehicle_id)
    const rivals = new Set([n?.ahead?.id, n?.behind?.id].filter((x): x is number => x !== undefined))
    const list = [...vehicles]
      .sort((a, b) => a.position - b.position)
      .map((v) => {
        const cls = v.vehicle_class || ''
        const rank = (rankInClass.get(cls) ?? 0) + 1
        rankInClass.set(cls, rank)
        return { id: v.id, color: getClassColor(cls), label: rank || v.position, isPlayer: v.id === player_vehicle_id, isRival: rivals.has(v.id) }
      })
    rosterCache.current = { key, list }
    return list
  })

  // Local yellows. In LMU `1` means yellow and `11` means clear — not `0`.
  const sectorFlagKey = useTelemetryStore((s) => s.vehicleStatus.sector_flags.join(','))
  const yellowSectors = useMemo(() => sectorFlagKey.split(',').map((v) => v === '1'), [sectorFlagKey])

  // --------------------------------------------------------------------------
  // State and refs
  // --------------------------------------------------------------------------

  const [outline, setOutline] = useState<TrackOutline | null>(null)
  const [recordState, setRecordState] = useState<'idle' | 'outlap' | 'recording'>('idle')
  const [bounds, setBounds] = useState<Bounds>(FALLBACK_BOUNDS)
  /** The lap being recorded, already projected. Written by the refit timer. */
  const [trailLine, setTrailLine] = useState('')

  /** One lap in progress per car; null once the outline is done. */
  const buildsRef = useRef<Map<number, Build> | null>(null)
  /** The live copy of `outline`, for the recorder and the bridge replies. */
  const outlineRef = useRef<TrackOutline | null>(null)
  const loadedTrackRef = useRef('')
  /** False until the view has been fitted to something real, so the placeholder
   *  bounds never end up merged into a track's own extent. */
  const fittedRef = useRef(false)
  /** The live copy of `bounds`, for the parts that run outside render: the
   *  animation loop reads it every frame, the refit timer grows it. */
  const boundsRef = useRef<Bounds>(FALLBACK_BOUNDS)

  const trailsRef = useRef(new Map<number, Trail>())
  const markersRef = useRef(new Map<number, SVGGElement>())
  const elevMarkerRef = useRef<SVGGElement | null>(null)
  const playerIdRef = useRef(-1)

  /** Returning the previous value tells React there is nothing to re-render,
   *  which matters because the recorder calls this on every sample. */
  const setRecord = useCallback((next: 'idle' | 'outlap' | 'recording') => {
    setRecordState((prev) => (prev === next ? prev : next))
  }, [])

  const applyBounds = useCallback((next: Bounds) => {
    if (boundsEqual(next, boundsRef.current)) return
    boundsRef.current = next
    setBounds(next)
  }, [])

  // --------------------------------------------------------------------------
  // Load the stored outline when the track changes
  // --------------------------------------------------------------------------
  useEffect(() => {
    if (!trackName || trackName === loadedTrackRef.current) return
    loadedTrackRef.current = trackName

    const saved = loadOutline(trackName)
    fittedRef.current = false
    outlineRef.current = saved
    setOutline(saved)
    setTrailLine('')
    setRecord('idle')

    // Record when there is nothing stored, and re-record when what is stored
    // predates the elevation and sector data. The old outline keeps drawing
    // until the new one is finished.
    buildsRef.current = needsRecording(saved) ? new Map() : null
    // The bridge may already have one, recorded by another dashboard.
    sendBridgeCommand({ command: 'track_outline_get', track: trackName })
  }, [trackName, setRecord])

  /** Show and keep an outline: in this browser, and on the bridge if it came
   *  from here. A complete one with its sector lines stops the recording. */
  const adoptOutline = useCallback((o: TrackOutline, upload: boolean) => {
    outlineRef.current = o
    setOutline(o)
    saveOutline(o)
    if (upload) sendBridgeCommand({ command: 'track_outline_save', track: o.trackName, json: JSON.stringify(o) })
    if (!needsRecording(o) && hasSectorLines(o)) {
      buildsRef.current = null
      setRecord('idle')
    }
  }, [setRecord])

  // --------------------------------------------------------------------------
  // The bridge's stored outline: take it if it is better than ours, give it
  // ours if ours is better. Asked again on every reconnect.
  // --------------------------------------------------------------------------
  useEffect(() => {
    const unsubReply = useTelemetryStore.subscribe((state, prev) => {
      const reply = state.trackOutline
      if (!reply || reply === prev.trackOutline || reply.track !== loadedTrackRef.current) return
      const theirs = parseOutline(reply.json)
      const ours = outlineRef.current
      if (theirs && theirs.trackName === reply.track && isBetterOutline(theirs, ours)) {
        adoptOutline(theirs, false)
      } else if (ours && isBetterOutline(ours, theirs)) {
        sendBridgeCommand({ command: 'track_outline_save', track: ours.trackName, json: JSON.stringify(ours) })
      }
    })
    // Deleted from the settings: forget it and record again.
    const unsubDel = useTelemetryStore.subscribe((state, prev) => {
      const del = state.trackOutlineDeleted
      if (!del || del === prev.trackOutlineDeleted || del.track !== loadedTrackRef.current) return
      outlineRef.current = null
      fittedRef.current = false
      setOutline(null)
      buildsRef.current = new Map()
      setRecord('outlap')
    })
    const unsubConn = useTelemetryStore.subscribe((state, prev) => {
      if (state.connection.status === 'connected' && prev.connection.status !== 'connected' && loadedTrackRef.current) {
        sendBridgeCommand({ command: 'track_outline_get', track: loadedTrackRef.current })
      }
    })
    return () => { unsubReply(); unsubDel(); unsubConn() }
  }, [adoptOutline, setRecord])

  // --------------------------------------------------------------------------
  // Sample every car's line into an outline: the first clean lap of any of
  // them makes the map, so it does not wait for ours or for our pit stops.
  // --------------------------------------------------------------------------
  useEffect(() => {
    const id = setInterval(() => {
      const builds = buildsRef.current
      if (!builds) return

      const { scoring, session } = useTelemetryStore.getState()
      const length = session.track_length
      const trackName = loadedTrackRef.current
      if (length <= 0 || !trackName) return

      let recording = false
      const present = new Set<number>()
      for (const car of scoring.vehicles) {
        if (car.pos_x === 0 && car.pos_z === 0) continue
        present.add(car.id)
        let build = builds.get(car.id)
        if (!build) {
          build = freshBuild(trackName)
          build.prevDist = car.lap_dist
          builds.set(car.id, build)
        }
        const done = stepBuild(build, car, length)
        if (done && isBetterOutline(done, outlineRef.current)) {
          adoptOutline(done, true)
          if (!buildsRef.current) return
        }
        if (build.crossings >= 1 && !build.broken) recording = true
      }
      for (const id of builds.keys()) if (!present.has(id)) builds.delete(id)
      // With an outline already on screen the rest is a quiet retry for the
      // sector lines: nothing to announce.
      setRecord(outlineRef.current?.complete ? 'idle' : recording ? 'recording' : 'outlap')
    }, RECORD_INTERVAL_MS)

    return () => clearInterval(id)
  }, [setRecord, adoptOutline])  // reads the store imperatively

  // --------------------------------------------------------------------------
  // Fit the view
  //
  // A finished outline pins the view to its own extent, which is what keeps the
  // map from drifting under the cars. Until there is one, the view grows to
  // hold the cars and the lap being recorded, and never shrinks again.
  // --------------------------------------------------------------------------
  useEffect(() => {
    if (outline?.complete) {
      fittedRef.current = true
      applyBounds(outline.bounds)
      setTrailLine('')
      return
    }

    const fit = () => {
      // The lap in progress furthest along, of any car.
      let pts: TrackPoint[] = []
      for (const b of buildsRef.current?.values() ?? []) {
        if (b.crossings >= 1 && !b.broken && b.points.length > pts.length) pts = b.points
      }
      const cars = useTelemetryStore.getState().scoring.vehicles
        .filter((v) => v.pos_x !== 0 || v.pos_z !== 0)
        .map((v) => ({ x: v.pos_x, z: v.pos_z }))
      if (!pts.length && !cars.length) return

      const seen = calcBounds([...pts, ...cars])
      applyBounds(fittedRef.current ? unionBounds(boundsRef.current, seen) : seen)
      fittedRef.current = true
      setTrailLine(pts.length >= 2 ? toPolyline(pts, boundsRef.current) : '')
    }

    fit()
    const id = setInterval(fit, REFIT_MS)
    return () => clearInterval(id)
  }, [outline, applyBounds])

  // --------------------------------------------------------------------------
  // Collect position samples — no render, straight into the motion buffers
  // --------------------------------------------------------------------------
  useEffect(() => {
    return useTelemetryStore.subscribe((state, prev) => {
      if (state.scoring.vehicles === prev.scoring.vehicles) return
      const now = performance.now()
      const trails = trailsRef.current
      playerIdRef.current = state.scoring.player_vehicle_id

      const live = new Set<number>()
      for (const v of state.scoring.vehicles) {
        live.add(v.id)
        let trail = trails.get(v.id)
        if (!trail) { trail = newTrail(); trails.set(v.id, trail) }
        pushSample(trail, now, v.pos_x, v.pos_z, v.lap_dist)
      }
      for (const id of trails.keys()) if (!live.has(id)) trails.delete(id)
    })
  }, [])

  // --------------------------------------------------------------------------
  // Move the markers, every frame
  // --------------------------------------------------------------------------
  const placeMarker = useCallback((node: SVGGElement, id: number, now: number) => {
    const trail = trailsRef.current.get(id)
    const p = trail ? sampleAt(trail, now - renderDelay(trail)) : null
    if (!p) {
      node.setAttribute('opacity', '0')   // no position yet — not at the origin
      return
    }
    const [sx, sy] = worldToSvg(p.x, p.z, boundsRef.current)
    node.setAttribute('transform', `translate(${sx.toFixed(2)} ${sy.toFixed(2)})`)
    node.setAttribute('opacity', '1')
  }, [])

  useEffect(() => {
    let frame = 0
    const tick = () => {
      frame = requestAnimationFrame(tick)
      const now = performance.now()
      for (const [id, node] of markersRef.current) placeMarker(node, id, now)

      const elev = elevMarkerRef.current
      if (!elev) return
      const trail = trailsRef.current.get(playerIdRef.current)
      const p = trail ? sampleAt(trail, now - renderDelay(trail)) : null
      const len = useTelemetryStore.getState().session.track_length
      // Lap distance runs negative down the pit lane, where a marker on an axis
      // of track distance would be meaningless.
      if (p && len > 0 && p.d >= 0) {
        elev.setAttribute('transform', `translate(${((p.d / len) * 100).toFixed(3)} 0)`)
        elev.setAttribute('opacity', '1')
      } else {
        elev.setAttribute('opacity', '0')
      }
    }
    frame = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(frame)
  }, [placeMarker])

  const attachMarker = useCallback((id: number, node: SVGGElement | null) => {
    if (node) {
      markersRef.current.set(id, node)
      placeMarker(node, id, performance.now())   // placed before its first paint
    } else {
      markersRef.current.delete(id)
    }
  }, [placeMarker])

  // --------------------------------------------------------------------------
  // Geometry, recomputed only when the outline or the view changes
  // --------------------------------------------------------------------------
  const sectorLines = useMemo(() => {
    const runs = outline && splitSectors(outline)
    return runs ? runs.map((run) => toPolyline(run, bounds)) : null
  }, [outline, bounds])

  const plainLine = useMemo(
    // A complete lap closes on itself across the line.
    () => (!sectorLines && outline && outline.points.length >= 2
      ? toPolyline(outline.complete ? [...outline.points, outline.points[0]] : outline.points, bounds)
      : ''),
    [sectorLines, outline, bounds],
  )

  const marks = useMemo(() => {
    if (!outline?.complete || !hasDetail(outline)) return null
    const pts = outline.points
    const at = (i: number, half: number) => (i >= 0 ? crossMark(pts, i, bounds, half) : null)
    return {
      start: at(0, 13),
      s1: at(indexAtDistance(pts, outline.sector1Dist), 10),
      s2: at(indexAtDistance(pts, outline.sector2Dist), 10),
    }
  }, [outline, bounds])

  const stats = useMemo(() => computeStats(outline, trackLength), [outline, trackLength])

  const elevation = useMemo(
    () => (detail === 'full' && outline ? elevationPath(outline, 100, ELEV_STRIP_H) : null),
    [detail, outline],
  )

  // --------------------------------------------------------------------------
  // Render
  // --------------------------------------------------------------------------
  const rich = detail !== 'compact'
  const roadWidth = outline?.complete ? 14 : 3
  const status = recordState === 'recording' ? ' — recording…'
    : recordState === 'outlap' ? ' — waiting for a car at the line…'
    : ''

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      {/* Header */}
      <div style={{
        display: 'flex', justifyContent: 'space-between', alignItems: 'baseline',
        paddingBottom: 4, borderBottom: `1px solid ${colors.border}`, marginBottom: 4, flexShrink: 0,
      }}>
        <span style={{ fontFamily: fonts.body, fontSize: 15, color: colors.textMuted, letterSpacing: 2, textTransform: 'uppercase' }}>
          Track Map
        </span>
        {trackName && (
          <span style={{ fontFamily: fonts.mono, fontSize: 15, color: colors.textMuted }}>
            {trackName}{status}
          </span>
        )}
      </div>

      {roster.length === 0 ? (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: colors.textMuted, fontFamily: fonts.body, fontSize: 15 }}>
          Waiting for session…
        </div>
      ) : (
        <div style={{ flex: 1, position: 'relative', minHeight: 0 }}>
          <svg
            viewBox={`0 0 ${SVG_SIZE} ${SVG_SIZE}`}
            style={{ width: '100%', height: '100%' }}
            preserveAspectRatio="xMidYMid meet"
          >
            {/* Road — split into its sectors once the boundaries are known, and
                shaded per sector unless the widget is set to compact */}
            {sectorLines ? sectorLines.map((pts, i) => (
              <polyline
                key={i}
                points={pts}
                fill="none"
                stroke={!rich ? TRACK_FILL : yellowSectors[i] ? SECTOR_YELLOW : SECTOR_FILL[i]}
                strokeWidth={roadWidth}
                strokeLinejoin="round"
                strokeLinecap="round"
              />
            )) : plainLine && (
              <polyline
                points={plainLine}
                fill="none"
                stroke={TRACK_FILL}
                strokeWidth={roadWidth}
                strokeLinejoin="round"
                strokeLinecap="round"
              />
            )}

            {/* The lap being recorded, drawn over whatever outline is stored */}
            {trailLine && (
              <polyline
                points={trailLine}
                fill="none"
                stroke={colors.primary}
                strokeWidth={2}
                strokeLinejoin="round"
                strokeLinecap="round"
                opacity={0.65}
              />
            )}

            {/* Start/finish and sector lines */}
            {rich && marks && (
              <g>
                {marks.s1 && <line {...marks.s1} stroke="#8a8a8a" strokeWidth={2} />}
                {marks.s2 && <line {...marks.s2} stroke="#8a8a8a" strokeWidth={2} />}
                {marks.start && <line {...marks.start} stroke="#e5e5e5" strokeWidth={3} />}
              </g>
            )}

            {/* Cars — placed by the animation loop, not by this render */}
            {roster.map((v) => {
              const r = v.isPlayer ? 13 : 10
              return (
                <g key={v.id} ref={(node) => attachMarker(v.id, node)} opacity={0}>
                  {v.isPlayer && (
                    <circle r={r + 5} fill="none" stroke={PLAYER_STROKE} strokeWidth={1.5} opacity={0.35} />
                  )}
                  {v.isRival && (
                    <circle r={r + 4} fill="none" stroke={RIVAL_STROKE} strokeWidth={2} strokeDasharray="3 2" />
                  )}
                  <circle
                    r={r}
                    fill={v.color}
                    stroke={v.isPlayer ? PLAYER_STROKE : 'rgba(0,0,0,0.4)'}
                    strokeWidth={v.isPlayer ? 2 : 1}
                    opacity={0.92}
                  />
                  <text
                    textAnchor="middle"
                    dominantBaseline="central"
                    fill="white"
                    fontSize={r > 11 ? 10 : 8}
                    fontWeight="bold"
                    fontFamily={fonts.body}
                    style={{ userSelect: 'none', pointerEvents: 'none' }}
                  >
                    {v.label}
                  </text>
                </g>
              )
            })}
          </svg>

          {/* Track data */}
          {rich && (trackLength > 0 || stats) && (
            <div style={{
              position: 'absolute', top: 0, left: 0,
              display: 'flex', flexDirection: 'column', gap: 1,
              fontFamily: fonts.mono, fontSize: 13, pointerEvents: 'none',
            }}>
              {trackLength > 0 && <StatRow label="LENGTH" value={km(trackLength)} />}
              {stats?.sectors && (
                <StatRow
                  label="SECTORS"
                  value={`${kmShort(stats.sectors[0])} / ${kmShort(stats.sectors[1])} / ${kmShort(stats.sectors[2])} km`}
                />
              )}
              {stats && <StatRow label="ELEV" value={`Δ ${metres(stats.elevRange)}   ↑ ${metres(stats.climb)}`} />}
              {stats && detail === 'full' && (
                <>
                  <StatRow label="RANGE" value={`${Math.round(stats.elevMin)} – ${Math.round(stats.elevMax)} m`} />
                  <StatRow label="GRADE" value={`${stats.maxGradient.toFixed(1)} % max`} />
                </>
              )}
            </div>
          )}

          {/* Class legend */}
          <div style={{
            position: 'absolute', bottom: 4, right: 6,
            display: 'flex', gap: 7, alignItems: 'center',
          }}>
            {([
              ['HC', CLASS_COLORS.Hypercar],
              ['GT3', CLASS_COLORS.LMGT3],
              ['LMP2', CLASS_COLORS.LMP2],
              ['LMP3', CLASS_COLORS.LMP3],
            ] as [string, string][]).map(([label, clr]) => (
              <span key={label} style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                <span style={{
                  width: 7, height: 7, borderRadius: '50%',
                  background: clr, display: 'inline-block', flexShrink: 0,
                }} />
                <span style={{ fontFamily: fonts.body, fontSize: 13, color: colors.textMuted }}>
                  {label}
                </span>
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Elevation over one lap, with the player's position on it */}
      {elevation && (
        <div style={{ flexShrink: 0, marginTop: 4, borderTop: `1px solid ${colors.border}`, paddingTop: 4 }}>
          <svg
            viewBox={`0 0 100 ${ELEV_STRIP_H}`}
            preserveAspectRatio="none"
            style={{ width: '100%', height: ELEV_STRIP_H, display: 'block' }}
          >
            <path d={elevation.area} fill={colors.primary} opacity={0.12} />
            <path d={elevation.line} fill="none" stroke={colors.primary} strokeWidth={1}
              opacity={0.7} vectorEffect="non-scaling-stroke" />
            {trackLength > 0 && [outline?.sector1Dist, outline?.sector2Dist].map((d, i) =>
              d && d > 0 ? (
                <line key={i} x1={(d / trackLength) * 100} y1={0} x2={(d / trackLength) * 100} y2={ELEV_STRIP_H}
                  stroke={colors.border} strokeWidth={1} vectorEffect="non-scaling-stroke" />
              ) : null,
            )}
            <g ref={elevMarkerRef} opacity={0}>
              <line x1={0} y1={0} x2={0} y2={ELEV_STRIP_H}
                stroke={PLAYER_STROKE} strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
            </g>
          </svg>
        </div>
      )}
    </div>
  )
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: 'flex', gap: 6 }}>
      <span style={{ width: 54, color: '#4b4b4b' }}>{label}</span>
      <span style={{ color: colors.textMuted }}>{value}</span>
    </div>
  )
}
