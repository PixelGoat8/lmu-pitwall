import { useState } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { sendBridgeCommand } from '../../hooks/useWebSocket'
import { colors, fonts } from '../../styles/theme'
import { fmtLap } from '../../utils/lapFormat'
import { useTeamStore } from '../../stores/teamStore'
import { useIsLocal } from '../../hooks/useIsLocal'
import { useAccess } from '../../hooks/useAccess'
import type { HubSourceStatus, HubStintInfo, TeamInfo } from '../../types/telemetry'

// Colours offered for a team: distinct from each other and from the class
// colours of the standings.
const TEAM_COLORS = ['#3987e5', '#d95926', '#199e70', '#c98500', '#9b59d0', '#d6457a', '#2bb3c0', '#8a8f98']

const LINK_TEXT: Record<string, string> = {
  connecting: 'connecting…',
  connected: 'connected',
  unauthorized: 'wrong token',
  unreachable: 'unreachable',
}

function linkColor(link: string): string {
  if (link === 'connected') return colors.success
  if (link === 'connecting') return colors.primary
  return colors.danger
}

/** What a bridge is doing, in a word. */
function activity(s: HubSourceStatus): { text: string; color: string } {
  if (s.link !== 'connected') return { text: LINK_TEXT[s.link] ?? s.link, color: linkColor(s.link) }
  if (!s.game_connected) return { text: 'game closed', color: colors.textMuted }
  if (!s.session_active) return { text: 'in menus', color: colors.textMuted }
  if (s.driving) return { text: s.in_pits ? 'driving · in pits' : 'driving', color: colors.success }
  return { text: 'watching', color: colors.textMuted }
}

function fmtClock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  return h > 0
    ? `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
    : `${m}:${String(sec).padStart(2, '0')}`
}

/**
 * The team as the hub sees it: each driver's bridge, who is being followed,
 * and the stint history. On an ordinary bridge it only explains what a hub is.
 */
export default function TeamWidget() {
  const status = useTelemetryStore((s) => s.hub.status)
  const stints = useTelemetryStore((s) => s.hub.stints)
  const error = useTelemetryStore((s) => s.hub.error)
  const sessionTime = useTelemetryStore((s) => s.scoring.session_time)
  const [link, setLink] = useState('')
  const [label, setLabel] = useState('')
  const teams = useTeamStore((s) => s.teams)
  const current = useTeamStore((s) => s.current)
  // Teams and drivers are managed on the hub's PC only (the hub refuses it
  // from elsewhere), so the controls appear only there.
  const local = useIsLocal()
  // Pinning a driver is the engineers'; a guest only watches.
  const { canChange } = useAccess()

  if (!status) {
    return (
      <div style={{ ...frame, justifyContent: 'center', color: colors.textMuted, fontFamily: fonts.body, fontSize: 14, lineHeight: 1.4 }}>
        <div style={{ color: colors.text, fontSize: 16 }}>Team — not a hub</div>
        This dashboard is connected to a single driver's Pitwall. To follow a whole
        team, start Pitwall with <code>--hub</code> on the engineer's PC and open
        that dashboard instead.
      </div>
    )
  }

  const add = () => {
    if (!link.trim()) return
    sendBridgeCommand({ command: 'hub_add_source', link: link.trim(), label: label.trim() || null })
    setLink('')
    setLabel('')
  }
  const follow = (id: string | null) => sendBridgeCommand({ command: 'hub_follow', id })
  const remove = (s: HubSourceStatus) => {
    if (window.confirm(`Remove ${s.label} from the hub?`)) sendBridgeCommand({ command: 'hub_remove_source', id: s.id })
  }
  const move = (s: HubSourceStatus, teamId: string) => {
    const to = teams.find((t) => t.id === teamId)
    if (to && window.confirm(`Move ${s.label} to ${to.name}?`)) sendBridgeCommand({ command: 'hub_move_source', source_id: s.id, team_id: teamId })
  }

  // One team, one car: drivers of this team in a session on different cars
  // means one of them belongs to another team, and the hub cannot follow both.
  const cars = new Map<string, string[]>()
  for (const s of status.sources) {
    if (s.link === 'connected' && s.car_name) cars.set(s.car_name, [...(cars.get(s.car_name) ?? []), s.label])
  }
  const mixedCars = cars.size > 1

  return (
    <div id="hub-team-panel" style={frame}>
      {/* Follow mode */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <span style={sectionTitle}>Following</span>
        <button onClick={() => follow(null)} disabled={!canChange} style={pill(status.follow === null)} title="Switch automatically to whoever is driving">
          Auto: the driver
        </button>
        {status.follow !== null ? (
          <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.accent }}>pinned — will not switch on a driver change</span>
        ) : null}
      </div>

      {/* Bridges */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        {status.sources.length === 0 ? (
          <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }}>
            No drivers yet. Ask each driver for the link behind the 🔗 button of their Pitwall and paste it below.
          </span>
        ) : status.sources.map((s) => {
          const act = activity(s)
          const isActive = s.id === status.active_id
          const isPinned = s.id === status.follow
          return (
            <div key={s.id} style={{
              display: 'flex', alignItems: 'center', gap: 8, padding: '5px 8px', borderRadius: 4,
              background: isActive ? `${colors.primary}18` : colors.bgWidget,
              border: `1px solid ${isActive ? colors.primary : colors.border}`,
            }}>
              <span style={{ width: 9, height: 9, borderRadius: '50%', background: linkColor(s.link), flexShrink: 0 }} />
              <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0, flex: 1 }}>
                <span style={{ fontFamily: fonts.body, fontSize: 16, color: colors.text, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                  {s.label}
                  {isActive ? <span style={{ color: colors.primary, fontSize: 12, letterSpacing: 1 }}> · ON SCREEN</span> : null}
                </span>
                <span style={{ fontFamily: fonts.body, fontSize: 12, color: act.color, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                  {act.text}
                  {s.session_active && s.driver_name && !s.driving ? ` · car driven by ${s.driver_name}` : ''}
                  <span style={{ color: colors.textMuted }}> · {s.host}</span>
                </span>
              </div>
              {canChange ? (
                <button onClick={() => follow(isPinned ? null : s.id)} style={pill(isPinned)} title={isPinned ? 'Back to following the driver' : 'Stay on this PC whoever drives'}>
                  {isPinned ? 'Pinned' : 'Pin'}
                </button>
              ) : isPinned ? <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.accent }}>pinned</span> : null}
              {local && teams.length > 1 && current ? (
                <select
                  value={current}
                  onChange={(e) => move(s, e.target.value)}
                  title="Move this driver to another team"
                  style={{ ...input, fontSize: 13, padding: '3px 4px', flex: '0 1 auto', maxWidth: 120 }}
                >
                  {teams.map((t) => <option key={t.id} value={t.id}>{t.id === current ? t.name : `→ ${t.name}`}</option>)}
                </select>
              ) : null}
              {local ? <button onClick={() => remove(s)} style={{ ...pill(false), padding: '3px 8px' }} title="Remove from the hub">✕</button> : null}
            </div>
          )
        })}
      </div>

      {mixedCars ? (
        <div style={{ fontFamily: fonts.body, fontSize: 13, color: colors.accent, border: `1px solid ${colors.accent}`, borderRadius: 4, padding: '5px 8px', lineHeight: 1.4 }}>
          ⚠ These drivers are on different cars:{' '}
          {[...cars].map(([car, who]) => `${who.join(', ')} on ${car}`).join(' · ')}.
          {' '}A team follows one car: move the other driver to their own team{local ? ' (menu next to the name)' : ' on the hub PC'}.
        </div>
      ) : null}

      {/* Add a bridge */}
      {local ? (
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
        <input
          value={link}
          onChange={(e) => setLink(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') add() }}
          placeholder="Driver's link: http://…:9000/?token=…"
          style={{ ...input, flex: '3 1 220px' }}
        />
        <input
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') add() }}
          placeholder="Name"
          style={{ ...input, flex: '1 1 80px' }}
        />
        <button onClick={add} disabled={!link.trim()} style={{ ...pill(true), opacity: link.trim() ? 1 : 0.4 }}>Add</button>
      </div>
      ) : (
        <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.textMuted }}>
          Drivers and teams are managed on the hub's PC.
        </span>
      )}
      {error ? <span style={{ fontFamily: fonts.body, fontSize: 13, color: colors.danger }}>{error}</span> : null}

      {local && teams.length > 0 ? <TeamsEditor teams={teams} current={current} /> : null}

      {/* Stints */}
      <span style={sectionTitle}>Stints</span>
      {stints.length === 0 ? (
        <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }}>No stint yet: the first one starts when the car leaves the pits.</span>
      ) : (
        <div style={{ overflow: 'auto', minHeight: 0 }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontFamily: fonts.mono, fontSize: 13, color: colors.text }}>
            <thead>
              <tr style={{ color: colors.textMuted, fontFamily: fonts.body, fontSize: 12, textTransform: 'uppercase', letterSpacing: 1 }}>
                {['#', 'Driver', 'Time', 'Laps', 'Best', 'Avg', 'Fuel/lap', 'VE/lap'].map((h, i) => (
                  <th key={h} style={{ textAlign: i < 2 ? 'left' : 'right', padding: '2px 6px', fontWeight: 400, whiteSpace: 'nowrap' }}>{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {[...stints].reverse().map((s) => <StintRow key={s.number} stint={s} now={sessionTime} />)}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}

/**
 * The hub's teams: rename, recolour, delete, create. Only on the hub's PC.
 * A team with drivers cannot be deleted, nor the last one; the hub says why.
 */
function TeamsEditor({ teams, current }: { teams: TeamInfo[]; current: string | null }) {
  const [name, setName] = useState('')
  const [palette, setPalette] = useState<string | null>(null)
  const create = () => {
    if (!name.trim()) return
    const used = new Set(teams.map((t) => t.color))
    const color = TEAM_COLORS.find((c) => !used.has(c))
    sendBridgeCommand({ command: 'team_create', name: name.trim(), color: color ?? null })
    setName('')
  }
  const rename = (t: TeamInfo, value: string) => {
    const v = value.trim()
    if (v && v !== t.name) sendBridgeCommand({ command: 'team_update', id: t.id, name: v })
  }
  const recolour = (t: TeamInfo, color: string) => {
    setPalette(null)
    if (color !== t.color) sendBridgeCommand({ command: 'team_update', id: t.id, color })
  }
  const remove = (t: TeamInfo) => {
    if (window.confirm(`Delete the team ${t.name}?`)) sendBridgeCommand({ command: 'team_delete', id: t.id })
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <span style={sectionTitle}>Teams</span>
      {teams.map((t) => {
        const canDelete = teams.length > 1 && t.sources === 0
        return (
          <div key={t.id} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <button
                onClick={() => setPalette(palette === t.id ? null : t.id)}
                title="Team colour"
                aria-label={`Colour of ${t.name}`}
                style={{ width: 22, height: 22, borderRadius: '50%', background: t.color, border: `2px solid ${palette === t.id ? colors.text : colors.border}`, cursor: 'pointer', flexShrink: 0, padding: 0 }}
              />
              <input
                key={t.name}
                defaultValue={t.name}
                onBlur={(e) => rename(t, e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') e.currentTarget.blur()
                  if (e.key === 'Escape') { e.currentTarget.value = t.name; e.currentTarget.blur() }
                }}
                aria-label="Team name"
                style={{ ...input, flex: '1 1 100px', fontWeight: t.id === current ? 600 : 400 }}
              />
              <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.textMuted, whiteSpace: 'nowrap' }}>
                {t.sources === 1 ? '1 driver' : `${t.sources} drivers`}
                {t.id === current ? ' · on screen' : ''}
              </span>
              <button
                onClick={() => remove(t)}
                disabled={!canDelete}
                title={canDelete ? 'Delete this team' : teams.length === 1 ? 'The last team cannot be deleted' : 'Move its drivers to another team first'}
                style={{ ...pill(false), padding: '3px 8px', opacity: canDelete ? 1 : 0.35, cursor: canDelete ? 'pointer' : 'default' }}
              >✕</button>
            </div>
            {palette === t.id ? (
              <div style={{ display: 'flex', gap: 6, paddingLeft: 30, flexWrap: 'wrap' }}>
                {TEAM_COLORS.map((c) => (
                  <button
                    key={c}
                    onClick={() => recolour(t, c)}
                    aria-label={c}
                    style={{ width: 20, height: 20, borderRadius: '50%', background: c, border: `2px solid ${c === t.color ? colors.text : 'transparent'}`, cursor: 'pointer', padding: 0 }}
                  />
                ))}
              </div>
            ) : null}
          </div>
        )
      })}
      <div style={{ display: 'flex', gap: 6 }}>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') create() }}
          placeholder="New team, e.g. Clienti"
          style={{ ...input, flex: 1 }}
        />
        <button onClick={create} disabled={!name.trim()} style={{ ...pill(true), opacity: name.trim() ? 1 : 0.4 }}>Create</button>
      </div>
    </div>
  )
}

function StintRow({ stint: s, now }: { stint: HubStintInfo; now: number }) {
  const running = s.end_time === null
  const duration = (running ? Math.max(now, s.start_time) : (s.end_time as number)) - s.start_time
  // Per-lap figures need laps and both ends of the stint.
  const perLap = (start: number | null, end: number | null) =>
    start !== null && end !== null && s.laps > 0 ? (start - end) / s.laps : null
  const fuel = perLap(s.fuel_start, s.fuel_end)
  const ve = perLap(s.ve_start, s.ve_end)
  const cell: React.CSSProperties = { textAlign: 'right', padding: '2px 6px', whiteSpace: 'nowrap' }
  return (
    <tr style={{ borderTop: `1px solid ${colors.border}55`, color: running ? colors.primary : colors.text }}>
      <td style={{ ...cell, textAlign: 'left' }}>{s.number}</td>
      <td style={{ ...cell, textAlign: 'left', fontFamily: fonts.body, fontSize: 14 }}>
        {s.driver}{running ? ' ●' : ''}
      </td>
      <td style={cell} title={`Session time ${fmtClock(s.start_time)} to ${running ? 'now' : fmtClock(s.end_time as number)}`}>{fmtClock(duration)}</td>
      <td style={cell} title={`Lap ${s.start_lap + 1} to ${s.end_lap}`}>{s.laps}</td>
      <td style={cell}>{s.best_lap !== null ? fmtLap(s.best_lap) : '—'}</td>
      <td style={cell}>{s.avg_lap !== null ? fmtLap(s.avg_lap) : '—'}</td>
      <td style={cell}>{fuel !== null ? `${fuel.toFixed(2)} L` : '—'}</td>
      <td style={cell}>{ve !== null ? `${(ve * 100).toFixed(1)}%` : '—'}</td>
    </tr>
  )
}

const frame: React.CSSProperties = {
  width: '100%',
  height: '100%',
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
  padding: '8px 10px',
  boxSizing: 'border-box',
  overflow: 'auto',
}

const sectionTitle: React.CSSProperties = {
  fontFamily: fonts.body,
  fontSize: 13,
  color: colors.textMuted,
  letterSpacing: 2,
  textTransform: 'uppercase',
}

const input: React.CSSProperties = {
  minWidth: 0,
  background: colors.bgWidget,
  border: `1px solid ${colors.border}`,
  color: colors.text,
  fontFamily: fonts.body,
  fontSize: 14,
  padding: '5px 8px',
  borderRadius: 3,
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
    whiteSpace: 'nowrap',
    flexShrink: 0,
  }
}
