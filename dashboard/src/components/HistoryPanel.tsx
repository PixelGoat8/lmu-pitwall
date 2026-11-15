import { useEffect, useMemo, useState } from 'react'
import { Bar, BarChart, CartesianGrid, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import { useTelemetryStore } from '../stores/telemetryStore'
import { sendBridgeCommand } from '../hooks/useWebSocket'
import { colors, fonts } from '../styles/theme'
import { fmtLap } from '../utils/lapFormat'
import type { CarSummary, SessionDetail, SessionSummary, StintRow } from '../types/history'

// Categorical slots for lines (validated for the dark surface: dataviz skill,
// scripts/validate_palette.js, all checks pass). Slot order is fixed: "us" is
// always slot 1, rivals take 2-4 in the order they were picked.
const SERIES = ['#3987e5', '#d95926', '#199e70', '#c98500']
const MAX_RIVALS = 3
const GRID = '#2a2a2a'
const AXIS_TEXT = '#a3a3a3'

function fmtDate(unix: number): string {
  const d = new Date(unix * 1000)
  return d.toLocaleString(undefined, { day: '2-digit', month: '2-digit', year: '2-digit', hour: '2-digit', minute: '2-digit' })
}

function fmtDuration(s: number | null): string {
  if (s === null || !isFinite(s)) return '—'
  const t = Math.max(0, Math.round(s))
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  return h > 0 ? `${h}h${String(m).padStart(2, '0')}` : `${m}:${String(t % 60).padStart(2, '0')}`
}

const lapTick = (v: number) => {
  const m = Math.floor(v / 60)
  return `${m}:${(v - m * 60).toFixed(1).padStart(4, '0')}`
}

/** Post-race analysis of the sessions the bridge (or hub) recorded. */
export default function HistoryPanel({ onClose }: { onClose: () => void }) {
  const sessions = useTelemetryStore((s) => s.history.sessions)
  const detail = useTelemetryStore((s) => s.history.detail)
  const error = useTelemetryStore((s) => s.history.error)
  const [selected, setSelected] = useState<number | null>(null)

  useEffect(() => { sendBridgeCommand({ command: 'history_list' }) }, [])
  useEffect(() => {
    if (selected !== null) sendBridgeCommand({ command: 'history_session', session_id: selected })
  }, [selected])

  return (
    <div style={{ flex: 1, minHeight: 0, display: 'grid', gridTemplateColumns: 'minmax(240px, 320px) 1fr', background: colors.bg }}>
      <div style={{ borderRight: `1px solid ${colors.border}`, overflow: 'auto', padding: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontFamily: fonts.heading, fontSize: 26, color: colors.primary, letterSpacing: 1 }}>HISTORY</span>
          <span style={{ flex: 1 }} />
          <button onClick={() => sendBridgeCommand({ command: 'history_list' })} style={btn(false)} title="Refresh">↻</button>
          <button onClick={onClose} style={btn(false)} title="Close">✕</button>
        </div>
        {error ? <span style={{ color: colors.danger, fontFamily: fonts.body, fontSize: 13 }}>{error}</span> : null}
        {sessions === null ? <span style={muted}>Loading…</span>
          : sessions.length === 0 ? <span style={muted}>No sessions recorded yet. A session appears here after its first completed lap.</span>
            : sessions.map((s) => <SessionItem key={s.id} s={s} active={s.id === selected} onClick={() => setSelected(s.id)} />)}
      </div>
      <div style={{ overflow: 'auto', padding: 12, minWidth: 0 }}>
        {detail && detail.session.id === selected ? <Detail d={detail} /> : <span style={muted}>Pick a session on the left.</span>}
      </div>
    </div>
  )
}

function SessionItem({ s, active, onClick }: { s: SessionSummary; active: boolean; onClick: () => void }) {
  return (
    <button onClick={onClick} style={{
      textAlign: 'left', background: active ? `${colors.primary}22` : colors.bgWidget, border: `1px solid ${active ? colors.primary : colors.border}`,
      borderRadius: 4, padding: '6px 8px', cursor: 'pointer', display: 'flex', flexDirection: 'column', gap: 2, color: colors.text,
    }}>
      <span style={{ fontFamily: fonts.body, fontSize: 15 }}>{s.track || 'Unknown track'} · {s.session_type}</span>
      <span style={{ fontFamily: fonts.mono, fontSize: 12, color: colors.textMuted }}>
        {fmtDate(s.started_at)} · {s.laps} laps{s.best_lap ? ` · best ${fmtLap(s.best_lap)}` : ''}
      </span>
      <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.textMuted, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {s.car_name}{s.followed ? ' (engineer seat)' : ''} · {s.drivers.join(', ') || '—'}
      </span>
    </button>
  )
}

function Detail({ d }: { d: SessionDetail }) {
  const s = d.session
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18, maxWidth: 1200 }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' }}>
        <span style={{ fontFamily: fonts.heading, fontSize: 30, color: colors.text }}>{s.track} · {s.session_type}</span>
        <span style={muted}>{fmtDate(s.started_at)} · {s.car_name} ({s.car_class}) · {s.laps} laps</span>
        <span style={{ flex: 1 }} />
        <button
          onClick={() => { if (window.confirm('Delete this session from the history? This cannot be undone.')) sendBridgeCommand({ command: 'history_delete', session_id: s.id }) }}
          style={btn(false)} title="Delete (only from the PC that recorded it)"
        >Delete</button>
      </div>
      {s.followed ? <span style={muted}>Recorded from the engineer seat: fuel and tyre wear were not available.</span> : null}
      <Stints stints={d.stints} />
      <LapChart d={d} />
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))', gap: 18 }}>
        <Events d={d} />
        <AiComments d={d} />
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Stints
// ---------------------------------------------------------------------------

function perLap(start: number | null, end: number | null, laps: number): number | null {
  return start !== null && end !== null && laps > 0 && start > end ? (start - end) / laps : null
}

function Stints({ stints }: { stints: StintRow[] }) {
  if (stints.length === 0) return <Section title="Stints"><span style={muted}>No stint recorded.</span></Section>
  // Stints of a lap or two are mostly the out-lap: they stay in the table but
  // would dwarf the others in the chart.
  const bars = stints
    .filter((st) => st.avg_lap !== null && st.laps >= 3)
    .map((st) => ({ name: `#${st.number} ${st.driver ?? ''}`, avg: st.avg_lap as number, best: st.best_lap }))
  const minAvg = bars.length ? Math.min(...bars.map((b) => b.avg)) : 0
  const maxAvg = bars.length ? Math.max(...bars.map((b) => b.avg)) : 0
  const cell: React.CSSProperties = { padding: '3px 8px', textAlign: 'right', whiteSpace: 'nowrap' }
  return (
    <Section title="Stints">
      <div style={{ overflowX: 'auto' }}>
        <table style={{ borderCollapse: 'collapse', width: '100%', fontFamily: fonts.mono, fontSize: 13, color: colors.text }}>
          <thead>
            <tr style={{ color: colors.textMuted, fontFamily: fonts.body, fontSize: 12 }}>
              {['#', 'Driver', 'Duration', 'Laps', 'Best', 'Average', 'Fuel/lap', 'VE/lap'].map((h, i) => (
                <th key={h} style={{ ...cell, textAlign: i < 2 ? 'left' : 'right', fontWeight: 400 }}>{h}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {stints.map((st) => {
              const fuel = perLap(st.fuel_start, st.fuel_end, st.laps)
              const ve = perLap(st.ve_start, st.ve_end, st.laps)
              return (
                <tr key={st.number} style={{ borderTop: `1px solid ${colors.border}55` }}>
                  <td style={{ ...cell, textAlign: 'left' }}>{st.number}</td>
                  <td style={{ ...cell, textAlign: 'left', fontFamily: fonts.body, fontSize: 14 }}>{st.driver ?? '—'}</td>
                  <td style={cell}>{fmtDuration(st.end_time !== null && st.start_time !== null ? st.end_time - st.start_time : null)}</td>
                  <td style={cell}>{st.laps}</td>
                  <td style={cell}>{st.best_lap ? fmtLap(st.best_lap) : '—'}</td>
                  <td style={cell}>{st.avg_lap ? fmtLap(st.avg_lap) : '—'}</td>
                  <td style={cell}>{fuel !== null ? `${fuel.toFixed(2)} L` : '—'}</td>
                  <td style={cell}>{ve !== null ? `${(ve * 100).toFixed(1)}%` : '—'}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
      {bars.length > 1 ? (
        <div style={{ height: 180, marginTop: 8 }}>
          <span style={{ ...muted, fontSize: 12 }}>Average lap per stint (stints of 3 laps or more, out-lap excluded)</span>
          <ResponsiveContainer width="100%" height="90%">
            <BarChart data={bars} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
              <CartesianGrid stroke={GRID} vertical={false} />
              <XAxis dataKey="name" tick={{ fill: AXIS_TEXT, fontSize: 12 }} axisLine={{ stroke: GRID }} tickLine={false} />
              <YAxis domain={[Math.floor(minAvg - 0.5), Math.ceil(maxAvg + 0.5)]} tickFormatter={lapTick} tick={{ fill: AXIS_TEXT, fontSize: 11 }} axisLine={false} tickLine={false} width={56} allowDataOverflow />
              <Tooltip cursor={{ fill: '#ffffff10' }} contentStyle={tooltipBox} formatter={(v) => [fmtLap(Number(v)), 'Average lap']} />
              <Bar dataKey="avg" fill={SERIES[0]} radius={[4, 4, 0, 0]} maxBarSize={48} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      ) : null}
    </Section>
  )
}

// ---------------------------------------------------------------------------
// Lap times: us and up to three rivals
// ---------------------------------------------------------------------------

function LapChart({ d }: { d: SessionDetail }) {
  const carLaps = useTelemetryStore((s) => s.history.carLaps)
  const [rivals, setRivals] = useState<number[]>([])
  const sid = d.session.id
  const ourCar = d.cars.find((c) => c.car_name === d.session.car_name)

  useEffect(() => {
    if (rivals.length) sendBridgeCommand({ command: 'history_car_laps', session_id: sid, car_ids: rivals })
  }, [rivals, sid])

  const { rows, domain } = useMemo(() => {
    const byLap = new Map<number, Record<string, number | null>>()
    const put = (lap: number, key: string, v: number | null) => {
      const r = byLap.get(lap) ?? { lap }
      r[key] = v
      byLap.set(lap, r)
    }
    // Laps through the pits are left out: they would flatten the rest.
    for (const l of d.laps) put(l.lap, 'us', l.lap_time && !l.pit && !l.caution ? l.lap_time : null)
    if (carLaps && carLaps.session_id === sid) {
      for (const l of carLaps.laps) if (rivals.includes(l.car_id)) put(l.lap, `c${l.car_id}`, l.lap_time && !l.pit ? l.lap_time : null)
    }
    const rows = [...byLap.values()].sort((a, b) => (a.lap as number) - (b.lap as number))
    const values = rows.flatMap((r) => Object.entries(r).filter(([k, v]) => k !== 'lap' && typeof v === 'number').map(([, v]) => v as number))
    const sorted = [...values].sort((a, b) => a - b)
    // Clip slow outliers (a spin, traffic) so the racing pace stays readable.
    const lo = sorted.length ? sorted[0] - 0.5 : 0
    const hi = sorted.length ? Math.min(sorted[sorted.length - 1], sorted[Math.floor(sorted.length * 0.9)] + 3) + 0.5 : 1
    return { rows, domain: [Math.floor(lo), Math.ceil(hi)] }
  }, [d.laps, carLaps, rivals, sid])

  const candidates: CarSummary[] = d.cars.filter((c) => c.car_id !== ourCar?.car_id)
  const toggle = (id: number) => setRivals((r) => r.includes(id) ? r.filter((x) => x !== id) : r.length >= MAX_RIVALS ? r : [...r, id])
  const name = (id: number) => d.cars.find((c) => c.car_id === id)?.car_name ?? `#${id}`

  return (
    <Section title="Lap times">
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <span style={{ ...muted, fontSize: 12 }}>Compare with (up to {MAX_RIVALS}):</span>
        <select value="" onChange={(e) => e.target.value && toggle(Number(e.target.value))} style={selectBox}>
          <option value="">+ add a car…</option>
          {candidates.filter((c) => !rivals.includes(c.car_id)).map((c) => (
            <option key={c.car_id} value={c.car_id}>
              {c.class ?? ''} {c.car_name ?? `#${c.car_id}`} — best {c.best_lap ? fmtLap(c.best_lap) : '—'}
            </option>
          ))}
        </select>
        {rivals.map((id, i) => (
          <button key={id} onClick={() => toggle(id)} style={{ ...btn(false), borderColor: SERIES[i + 1] }} title="Remove">
            <span style={{ display: 'inline-block', width: 10, height: 3, background: SERIES[i + 1], marginRight: 6, verticalAlign: 'middle' }} />
            {name(id)} ✕
          </button>
        ))}
      </div>
      <div style={{ height: 300 }}>
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={rows} margin={{ top: 10, right: 16, bottom: 0, left: 0 }}>
            <CartesianGrid stroke={GRID} vertical={false} />
            <XAxis dataKey="lap" tick={{ fill: AXIS_TEXT, fontSize: 12 }} axisLine={{ stroke: GRID }} tickLine={false} label={{ value: 'lap', position: 'insideBottomRight', fill: AXIS_TEXT, fontSize: 11, offset: -2 }} />
            <YAxis domain={domain} tickFormatter={lapTick} tick={{ fill: AXIS_TEXT, fontSize: 11 }} axisLine={false} tickLine={false} width={56} allowDataOverflow />
            <Tooltip contentStyle={tooltipBox} labelFormatter={(l) => `Lap ${l}`} formatter={(v, key) => [fmtLap(Number(v)), String(key) === 'us' ? 'Our car' : name(Number(String(key).slice(1)))]} />
            <Legend formatter={(key: string) => (key === 'us' ? 'Our car' : name(Number(key.slice(1))))} wrapperStyle={{ fontFamily: fonts.body, fontSize: 13, color: colors.text }} />
            <Line dataKey="us" stroke={SERIES[0]} strokeWidth={2} dot={false} connectNulls={false} isAnimationActive={false} />
            {rivals.map((id, i) => (
              <Line key={id} dataKey={`c${id}`} stroke={SERIES[i + 1]} strokeWidth={2} dot={false} connectNulls={false} isAnimationActive={false} />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </div>
      <span style={{ ...muted, fontSize: 12 }}>Pit and caution laps are left out; very slow laps are clipped at the top.</span>
    </Section>
  )
}

// ---------------------------------------------------------------------------

function Events({ d }: { d: SessionDetail }) {
  return (
    <Section title={`Events (${d.events.length})`}>
      <div style={{ maxHeight: 280, overflow: 'auto', display: 'flex', flexDirection: 'column', gap: 2 }}>
        {d.events.length === 0 ? <span style={muted}>None.</span> : d.events.map((e, i) => (
          <span key={i} style={{ fontFamily: fonts.body, fontSize: 13, color: e.severity === 'alert' ? colors.danger : e.severity === 'warn' ? colors.accent : colors.text, borderLeft: `3px solid ${e.own ? colors.primary : 'transparent'}`, paddingLeft: 6 }}>
            <span style={{ fontFamily: fonts.mono, color: colors.textMuted }}>L{e.lap ?? '?'} </span>{e.text}
          </span>
        ))}
      </div>
    </Section>
  )
}

function AiComments({ d }: { d: SessionDetail }) {
  const cost = d.ai.reduce((a, c) => a + (c.cost_usd ?? 0), 0)
  return (
    <Section title={`AI comments (${d.ai.length}${d.ai.length ? `, $${cost.toFixed(3)}` : ''})`}>
      <div style={{ maxHeight: 280, overflow: 'auto', display: 'flex', flexDirection: 'column', gap: 6 }}>
        {d.ai.length === 0 ? <span style={muted}>None.</span> : d.ai.map((c, i) => (
          <div key={i} style={{ fontFamily: fonts.body, fontSize: 13, color: colors.text, borderLeft: `3px solid ${c.withheld ? colors.danger : colors.primary}`, paddingLeft: 6 }}>
            <span style={{ fontFamily: fonts.mono, color: colors.textMuted, fontSize: 11 }}>L{c.lap ?? '?'} · {c.trigger}{c.question ? ` · “${c.question}”` : ''}</span>
            <div>{c.withheld ? <span style={{ color: colors.danger }}>Withheld (numbers not in the data: {c.unverified})</span> : c.text}</div>
          </div>
        ))}
      </div>
    </Section>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <span style={{ fontFamily: fonts.body, fontSize: 13, color: colors.textMuted, letterSpacing: 2, textTransform: 'uppercase' }}>{title}</span>
      {children}
    </div>
  )
}

const muted: React.CSSProperties = { fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }
const tooltipBox: React.CSSProperties = { background: '#1a1a1a', border: `1px solid ${GRID}`, fontFamily: fonts.mono, fontSize: 12, color: colors.text }
const selectBox: React.CSSProperties = { background: colors.bgWidget, border: `1px solid ${colors.border}`, color: colors.text, fontFamily: fonts.body, fontSize: 14, padding: '4px 6px', borderRadius: 3, maxWidth: 360 }

function btn(active: boolean): React.CSSProperties {
  return {
    background: active ? colors.primary : colors.bgWidget, color: active ? '#000' : colors.text,
    border: `1px solid ${active ? colors.primary : colors.border}`, borderRadius: 3, fontFamily: fonts.body,
    fontSize: 13, padding: '4px 10px', cursor: 'pointer', whiteSpace: 'nowrap',
  }
}
