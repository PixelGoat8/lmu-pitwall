import { useTelemetryStore } from '../../stores/telemetryStore'
import { colors, fonts } from '../../styles/theme'

function fmtTime(unix: number): string {
  return new Date(unix * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
}

/**
 * The team's change log, newest first: who changed the strategy settings,
 * the targets, the pinned PC or the AI, and when ("21:14 Marco: VE target
 * 3.2 %/lap"). With two engineers on the same car, this is how each knows
 * what the other did.
 */
export default function ChangesWidget() {
  const changes = useTelemetryStore((s) => s.changes)
  const shown = [...(changes ?? [])].reverse()
  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column', gap: 4, padding: 8, overflow: 'auto', boxSizing: 'border-box' }}>
      {shown.length === 0 ? (
        <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }}>
          {changes === null ? 'No connection to the bridge' : 'No changes yet. Strategy settings, targets, Pin and the AI are logged here with who made them.'}
        </span>
      ) : shown.map((c) => (
        <div key={c.seq} style={{ display: 'flex', gap: 8, alignItems: 'baseline', fontFamily: fonts.body, fontSize: 15, color: colors.text, lineHeight: 1.3 }}>
          <span style={{ fontFamily: fonts.mono, fontSize: 12, color: colors.textMuted, whiteSpace: 'nowrap' }}>
            {fmtTime(c.at)}{c.lap !== null ? ` L${c.lap}` : ''}
          </span>
          <span><b>{c.who}</b>: {c.text}</span>
        </div>
      ))}
    </div>
  )
}
