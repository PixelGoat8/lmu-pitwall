import { useMemo } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { colors, fonts } from '../../styles/theme'
import { getClassColor } from '../../utils/classColors'
import { traffic, TRAFFIC_WINDOW_S } from '../../utils/traffic'
import type { TrafficCar } from '../../utils/traffic'
import { useSisterCars } from '../../utils/sisterCars'
import type { SisterCar } from '../../utils/sisterCars'

/** Show at most this many cars per side. */
const MAX_ROWS = 3
/** Below this the car is "on us": highlight. */
const URGENT_S = 5

/**
 * Multiclass traffic around our car: faster classes coming up to lap us, and
 * slower classes we are about to lap, with seconds until they meet at the
 * pace each has shown. Plus the blue flag when the game shows it to our driver.
 */
export default function TrafficWidget() {
  const vehicles = useTelemetryStore((s) => s.scoring.vehicles)
  const playerId = useTelemetryStore((s) => s.scoring.player_vehicle_id)
  const trackLength = useTelemetryStore((s) => s.session.track_length)
  const blue = useTelemetryStore((s) => s.vehicleStatus.player_flag === 6)
  const sisters = useSisterCars()

  const cars = useMemo(() => traffic(vehicles, playerId, trackLength), [vehicles, playerId, trackLength])
  const behind = cars.filter((c) => c.side === 'behind').slice(0, MAX_ROWS)
  const ahead = cars.filter((c) => c.side === 'ahead').slice(0, MAX_ROWS)

  return (
    <div style={frame}>
      {blue ? (
        <div style={{ background: '#2563eb', color: '#fff', fontFamily: fonts.heading, fontSize: 26, letterSpacing: 2, textAlign: 'center', borderRadius: 4, padding: '2px 0' }}>
          BLUE FLAG
        </div>
      ) : null}
      <Section title="Faster cars coming" empty="nobody closing from behind" rows={behind} sisters={sisters} />
      <Section title="Traffic ahead to lap" empty="no slower car within reach" rows={ahead} sisters={sisters} />
      <span style={{ fontFamily: fonts.body, fontSize: 11, color: colors.textMuted }}>
        Other classes within {TRAFFIC_WINDOW_S} s on track; time to catch at each car's recent pace.
      </span>
    </div>
  )
}

function Section({ title, empty, rows, sisters }: { title: string; empty: string; rows: TrafficCar[]; sisters: Map<string, SisterCar> }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
      <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.textMuted, letterSpacing: 2, textTransform: 'uppercase' }}>{title}</span>
      {rows.length === 0 ? (
        <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }}>{empty}</span>
      ) : rows.map((r) => <Row key={r.car.id} r={r} sister={sisters.get(r.car.car_name)} />)}
    </div>
  )
}

function Row({ r, sister }: { r: TrafficCar; sister?: SisterCar }) {
  const urgent = (r.catch_in_s ?? Infinity) < URGENT_S
  const cls = getClassColor(r.car.vehicle_class)
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '3px 6px', borderRadius: 3, background: urgent ? `${colors.accent}33` : 'transparent', border: `1px solid ${urgent ? colors.accent : 'transparent'}` }}>
      <span style={{ fontFamily: fonts.mono, fontSize: 12, color: cls, border: `1px solid ${cls}`, borderRadius: 3, padding: '0 4px', flexShrink: 0 }}>{r.car.vehicle_class}</span>
      <span style={{ fontFamily: fonts.body, fontSize: 15, color: colors.text, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', flex: 1, minWidth: 0 }}>
        #{r.car.id} {r.car.driver_name}
        {sister ? <span style={{ color: sister.color }}> · {sister.team}</span> : null}
      </span>
      <span style={{ fontFamily: fonts.mono, fontSize: 13, color: colors.textMuted, whiteSpace: 'nowrap' }}>{r.gap_s.toFixed(1)} s</span>
      <span style={{ fontFamily: fonts.mono, fontSize: 17, fontWeight: 700, color: urgent ? colors.accent : colors.text, whiteSpace: 'nowrap', minWidth: 64, textAlign: 'right' }}>
        {r.catch_in_s === null ? '—' : r.catch_in_s < 1 ? 'now' : `${Math.round(r.catch_in_s)} s`}
      </span>
    </div>
  )
}

const frame: React.CSSProperties = {
  width: '100%',
  height: '100%',
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
  padding: '6px 10px',
  boxSizing: 'border-box',
  overflow: 'auto',
}
