import { useState } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { colors, fonts } from '../../styles/theme'
import type { RaceEvent } from '../../types/race'

type Filter = 'all' | 'ours'

const SEVERITY_COLOR: Record<string, string> = {
  info: colors.textMuted,
  warn: colors.accent,
  alert: colors.danger,
}

const ICON: Record<string, string> = {
  fcy: '🟨', fcy_end: '🟩', pits: '🅿', start: '🟢', finish: '🏁', pit_in: '↘', pit_out: '↗',
  penalty: '⚠', driver: '👤', position: '⇅', impact: '💥', blue_flag: '🟦', best_lap: '⏱', class_best: '⏱',
}

function fmtClock(s: number): string {
  const t = Math.max(0, Math.floor(s))
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  const sec = t % 60
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}` : `${m}:${String(sec).padStart(2, '0')}`
}

/** The race log from the bridge, newest first. */
export default function EventsWidget() {
  const events = useTelemetryStore((s) => s.raceTiming?.events)
  const [filter, setFilter] = useState<Filter>('all')

  const shown = (events ?? [])
    .filter((e) => filter === 'all' || e.own || e.car_id === null)
    .slice()
    .reverse()

  return (
    <div style={frame}>
      <div style={{ display: 'flex', gap: 6 }}>
        {(['all', 'ours'] as const).map((f) => (
          <button key={f} onClick={() => setFilter(f)} style={pill(filter === f)}>
            {f === 'all' ? 'All' : 'Our car + race'}
          </button>
        ))}
      </div>
      {shown.length === 0 ? (
        <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }}>No events yet.</span>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2, overflow: 'auto', minHeight: 0 }}>
          {shown.map((e, i) => <Row key={`${e.t}-${e.kind}-${i}`} e={e} />)}
        </div>
      )}
    </div>
  )
}

function Row({ e }: { e: RaceEvent }) {
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'baseline', padding: '2px 4px', borderLeft: `3px solid ${e.own ? colors.primary : 'transparent'}` }}>
      <span style={{ fontFamily: fonts.mono, fontSize: 12, color: colors.textMuted, whiteSpace: 'nowrap', minWidth: 74 }}>
        {fmtClock(e.t)} L{e.lap}
      </span>
      <span style={{ width: 18, textAlign: 'center', flexShrink: 0 }}>{ICON[e.kind] ?? '•'}</span>
      <span style={{ fontFamily: fonts.body, fontSize: 14, color: e.severity === 'info' ? colors.text : SEVERITY_COLOR[e.severity] ?? colors.text }}>
        {e.text}
      </span>
    </div>
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
  overflow: 'hidden',
}

function pill(active: boolean): React.CSSProperties {
  return {
    background: active ? colors.primary : colors.bgWidget,
    color: active ? '#000' : colors.textMuted,
    border: `1px solid ${active ? colors.primary : colors.border}`,
    borderRadius: 3,
    fontFamily: fonts.body,
    fontSize: 13,
    padding: '3px 10px',
    cursor: 'pointer',
  }
}
