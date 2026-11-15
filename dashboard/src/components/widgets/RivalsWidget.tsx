import { useMemo } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { colors, fonts } from '../../styles/theme'
import { fmtLap } from '../../utils/lapFormat'
import { classNeighbours } from '../../utils/gaps'
import type { LapTime } from '../../types/race'
import type { VehicleScoring } from '../../types/telemetry'

/** Laps shown per car. */
const SHOWN = 6
/** Average over this many recent green laps. */
const AVG_OVER = 3

interface Column {
  label: string
  car: VehicleScoring
  laps: LapTime[]
  avg: number | null
}

/** Average of the most recent laps that did not go through the pits. */
function recentAvg(laps: LapTime[]): number | null {
  const green = laps.filter((l) => !l.pit && l.time > 0).slice(-AVG_OVER)
  return green.length ? green.reduce((a, l) => a + l.time, 0) / green.length : null
}

/**
 * Our last laps side by side with the class rivals directly ahead and behind
 * (and the class leader when it is someone else), each with the difference
 * to ours. Lap history comes from the bridge, so it survives a page reload.
 */
export default function RivalsWidget() {
  const vehicles = useTelemetryStore((s) => s.scoring.vehicles)
  const playerId = useTelemetryStore((s) => s.scoring.player_vehicle_id)
  const lapLog = useTelemetryStore((s) => s.raceTiming?.laps)

  const columns = useMemo<Column[]>(() => {
    const n = classNeighbours(vehicles, playerId)
    if (!n) return []
    const byId = new Map((lapLog ?? []).map((c) => [c.id, c.laps]))
    const col = (label: string, car: VehicleScoring): Column => {
      const laps = byId.get(car.id) ?? []
      return { label, car, laps, avg: recentAvg(laps) }
    }
    const leader = vehicles
      .filter((v) => v.vehicle_class === n.player.vehicle_class)
      .sort((a, b) => a.position - b.position)[0]
    const cols = [col('Us', n.player)]
    if (n.ahead) cols.push(col(`P${n.classPosition - 1} ahead`, n.ahead))
    if (n.behind) cols.push(col(`P${n.classPosition + 1} behind`, n.behind))
    if (leader && leader.id !== n.player.id && leader.id !== n.ahead?.id) cols.push(col('Class leader', leader))
    return cols
  }, [vehicles, playerId, lapLog])

  if (!columns.length) {
    return <div style={{ ...frame, justifyContent: 'center', alignItems: 'center', color: colors.textMuted, fontFamily: fonts.body, fontSize: 15 }}>Rivals — waiting for a session</div>
  }

  const us = columns[0]
  // Rows by lap number of our car's laps, newest first.
  const lapNumbers = us.laps.slice(-SHOWN).map((l) => l.lap).reverse()

  const cell: React.CSSProperties = { padding: '2px 6px', textAlign: 'right', whiteSpace: 'nowrap', fontFamily: fonts.mono, fontSize: 13 }
  return (
    <div style={frame}>
      <table style={{ borderCollapse: 'collapse', width: '100%', color: colors.text }}>
        <thead>
          <tr>
            <th style={{ ...cell, textAlign: 'left', fontFamily: fonts.body, color: colors.textMuted, fontWeight: 400 }}>Lap</th>
            {columns.map((c) => (
              <th key={c.car.id} style={{ ...cell, fontFamily: fonts.body, fontWeight: 400, color: c === us ? colors.primary : colors.textMuted }} title={c.car.driver_name}>
                {c.label}
                <div style={{ fontSize: 12, color: colors.text, maxWidth: 120, overflow: 'hidden', textOverflow: 'ellipsis' }}>#{c.car.id} {c.car.driver_name}</div>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          <tr style={{ borderBottom: `1px solid ${colors.border}` }}>
            <td style={{ ...cell, textAlign: 'left', fontFamily: fonts.body, color: colors.textMuted }}>avg {AVG_OVER}</td>
            {columns.map((c) => <td key={c.car.id} style={{ ...cell, fontWeight: 700 }}><Diff t={c.avg} ref_={c === us ? null : us.avg} /></td>)}
          </tr>
          {lapNumbers.map((lap) => {
            const mine = us.laps.find((l) => l.lap === lap)
            return (
              <tr key={lap} style={{ borderTop: `1px solid ${colors.border}44` }}>
                <td style={{ ...cell, textAlign: 'left', color: colors.textMuted }}>{lap}</td>
                {columns.map((c) => {
                  // The rival's lap completed closest to ours: same lap number
                  // if on the same lap, else their latest at that point.
                  const theirs = c.laps.find((l) => l.lap === lap)
                  return (
                    <td key={c.car.id} style={{ ...cell, color: theirs?.pit ? colors.textMuted : colors.text }}>
                      {theirs ? <Diff t={theirs.time} ref_={c === us || !mine || mine.pit || theirs.pit ? null : mine.time} pit={theirs.pit} /> : '—'}
                    </td>
                  )
                })}
              </tr>
            )
          })}
        </tbody>
      </table>
      <span style={{ fontFamily: fonts.body, fontSize: 11, color: colors.textMuted }}>
        Differences are to our lap with the same number: green = they were slower, red = faster. Pit laps in grey.
      </span>
    </div>
  )
}

function Diff({ t, ref_, pit = false }: { t: number | null; ref_: number | null; pit?: boolean }) {
  if (t === null) return <>—</>
  if (ref_ === null) return <>{fmtLap(t)}{pit ? ' P' : ''}</>
  const d = t - ref_
  return (
    <>
      {fmtLap(t)}
      <span style={{ color: d > 0 ? colors.success : colors.danger, fontSize: 12 }}> {d > 0 ? '+' : '−'}{Math.abs(d).toFixed(2)}</span>
    </>
  )
}

const frame: React.CSSProperties = {
  width: '100%',
  height: '100%',
  display: 'flex',
  flexDirection: 'column',
  gap: 6,
  padding: '6px 8px',
  boxSizing: 'border-box',
  overflow: 'auto',
}
