import { useEffect, useMemo, useRef, useState } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { colors, fonts } from '../../styles/theme'
import { getClassColor } from '../../utils/classColors'
import { classNeighbours, fmtGap, gapBetween, gapTrend } from '../../utils/gaps'
import type { Gap, GapSample } from '../../utils/gaps'
import type { VehicleScoring } from '../../types/telemetry'

// Gap history is kept per rival for a little over two laps of a long circuit.
const HISTORY_S = 600
// One sample a second is plenty: the underlying gaps change at timing points.
const SAMPLE_EVERY_S = 1
// Trend window when the player has no lap time yet.
const FALLBACK_WINDOW_S = 90
// Below this a change is noise, not a trend.
const TREND_DEADBAND_S = 0.05

type Side = 'ahead' | 'behind'

interface RowData {
  side: Side
  car: VehicleScoring | null
  classPos: number
  gap: Gap
  trend: number | null
}

/**
 * Gap to the cars one place ahead and behind in the player's class, with the
 * change over the last lap. Built for the engineer: two big numbers and
 * whether each is getting better or worse.
 */
export default function GapWidget() {
  const vehicles = useTelemetryStore((s) => s.scoring.vehicles)
  const playerId = useTelemetryStore((s) => s.scoring.player_vehicle_id)
  const sessionType = useTelemetryStore((s) => s.scoring.session_type)

  const isRace = sessionType?.toLowerCase().includes('race') ?? false
  const info = useMemo(() => classNeighbours(vehicles, playerId), [vehicles, playerId])

  // rival id → samples of the gap to that rival
  const historyRef = useRef<Map<number, GapSample[]>>(new Map())
  const lastSampleRef = useRef(-Infinity)
  const [trends, setTrends] = useState<{ ahead: number | null; behind: number | null }>({ ahead: null, behind: null })

  // Sample the gaps once a second of session time, straight from the store.
  useEffect(() => {
    return useTelemetryStore.subscribe((state) => {
      const sc = state.scoring
      const now = sc.session_time
      // The session clock going backwards is a new session: start over.
      if (now < lastSampleRef.current - 5) {
        historyRef.current.clear()
        lastSampleRef.current = -Infinity
      }
      if (now - lastSampleRef.current < SAMPLE_EVERY_S) return
      lastSampleRef.current = now

      const next: { ahead: number | null; behind: number | null } = { ahead: null, behind: null }
      const n = classNeighbours(sc.vehicles, sc.player_vehicle_id)
      if (n && (sc.session_type?.toLowerCase().includes('race') ?? false)) {
        const windowS = n.player.last_lap_time > 0 ? n.player.last_lap_time : FALLBACK_WINDOW_S
        for (const side of ['ahead', 'behind'] as const) {
          const car = n[side]
          if (!car) continue
          const gap = gapBetween(n.player, car, true)
          // A gap in laps has no meaningful lap-over-lap change; neither has
          // a gap measured while either car is in the pits.
          if (!gap || gap.kind !== 'time' || car.in_pits || n.player.in_pits) continue
          const list = historyRef.current.get(car.id) ?? []
          list.push({ t: now, seconds: gap.seconds })
          while (list.length && now - list[0].t > HISTORY_S) list.shift()
          historyRef.current.set(car.id, list)
          next[side] = gapTrend(list, now, windowS)
        }
      }
      setTrends((prev) => (prev.ahead === next.ahead && prev.behind === next.behind ? prev : next))
    })
  }, [])

  if (!info) {
    return (
      <div style={{ ...frame, alignItems: 'center', justifyContent: 'center', color: colors.textMuted, fontFamily: fonts.body, fontSize: 15 }}>
        Gaps — waiting for a session
      </div>
    )
  }

  const rows: RowData[] = [
    { side: 'ahead', car: info.ahead, classPos: info.classPosition - 1, gap: info.ahead ? gapBetween(info.player, info.ahead, isRace) : null, trend: trends.ahead },
    { side: 'behind', car: info.behind, classPos: info.classPosition + 1, gap: info.behind ? gapBetween(info.player, info.behind, isRace) : null, trend: trends.behind },
  ]
  const classColor = getClassColor(info.player.vehicle_class)

  return (
    <div style={frame}>
      {rows.map((r) => <GapRow key={r.side} row={r} isRace={isRace} />)}
      <div style={{ fontFamily: fonts.mono, fontSize: 12, color: classColor, whiteSpace: 'nowrap', textAlign: 'right', lineHeight: 1 }}>
        {info.player.vehicle_class} P{info.classPosition}/{info.classSize}{isRace ? '' : ' · gaps on best lap'}
      </div>
    </div>
  )
}

const frame: React.CSSProperties = {
  width: '100%',
  height: '100%',
  display: 'flex',
  flexDirection: 'column',
  justifyContent: 'center',
  gap: 4,
  padding: '4px 10px',
  boxSizing: 'border-box',
  overflow: 'hidden',
}

function GapRow({ row, isRace }: { row: RowData; isRace: boolean }) {
  const { side, car, gap, trend } = row
  const arrow = side === 'ahead' ? '▲' : '▼'

  // Good news is green on both rows: closing on the car ahead, pulling away
  // from the car behind.
  let trendText = ''
  let trendColor: string = colors.textMuted
  if (isRace && trend !== null) {
    if (Math.abs(trend) < TREND_DEADBAND_S) {
      trendText = 'steady'
    } else {
      const shrinking = trend < 0
      const good = side === 'ahead' ? shrinking : !shrinking
      trendColor = good ? colors.success : colors.danger
      const word = side === 'ahead'
        ? (shrinking ? 'closing' : 'losing')
        : (shrinking ? 'being caught' : 'pulling away')
      trendText = `${word} ${Math.abs(trend).toFixed(2)}/lap`
    }
  }

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
      <span style={{ fontFamily: fonts.body, fontSize: 13, color: colors.textMuted, width: 78, flexShrink: 0, letterSpacing: 1, textTransform: 'uppercase', whiteSpace: 'nowrap' }}>
        {arrow} {side}
      </span>
      {car ? (
        <>
          <span style={{ fontFamily: fonts.mono, fontSize: 24, fontWeight: 700, color: colors.text, lineHeight: 1, whiteSpace: 'nowrap', flexShrink: 0, minWidth: 92 }}>
            {fmtGap(gap)}
          </span>
          <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0, gap: 1 }}>
            <span style={{ fontFamily: fonts.body, fontSize: 15, color: colors.text, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', lineHeight: 1.1 }}>
              P{row.classPos} {car.driver_name}
              {car.in_pits ? <span style={{ color: colors.accent }}> · PIT</span> : null}
            </span>
            <span style={{ fontFamily: fonts.body, fontSize: 13, color: trendColor, whiteSpace: 'nowrap', minHeight: 14, lineHeight: 1 }}>
              {trendText}
            </span>
          </div>
        </>
      ) : (
        <span style={{ fontFamily: fonts.body, fontSize: 15, color: colors.textMuted }}>
          {side === 'ahead' ? 'leading the class' : 'last in class'}
        </span>
      )}
    </div>
  )
}
