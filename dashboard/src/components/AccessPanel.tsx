import { useEffect, useState } from 'react'
import { useTelemetryStore } from '../stores/telemetryStore'
import { useTeamStore } from '../stores/teamStore'
import { sendBridgeCommand } from '../hooks/useWebSocket'
import { bridgeApiUrl } from '../utils/bridge'
import { colors, fonts } from '../styles/theme'
import { LinkRow } from './Toolbar'
import type { OnlineInfo, PersonInfo, TeamInfo } from '../types/telemetry'

/** How often the open panel asks who is connected. */
const REFRESH_MS = 2000

interface NetworkInfo { ip: string; port: number; tailscale_ip?: string | null }

const ROLE_TEXT: Record<string, string> = { admin: 'hub PC', engineer: 'engineer', guest: 'guest' }

function fmtSince(s: number): string {
  if (s < 60) return `${s} s`
  const m = Math.floor(s / 60)
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, '0')}`
}

/**
 * The hub's people, on the hub's PC: add someone and copy their link, change
 * their role and teams, give them a new link, take their access away, and see
 * who is connected now. The hub refuses all of this from anywhere else.
 */
export default function AccessPanel({ onClose }: { onClose: () => void }) {
  const { people, online, error } = useTelemetryStore((s) => s.access)
  const teams = useTeamStore((s) => s.teams)
  const [net, setNet] = useState<NetworkInfo | null>(null)

  useEffect(() => {
    sendBridgeCommand({ command: 'access_list' })
    const t = setInterval(() => sendBridgeCommand({ command: 'access_list' }), REFRESH_MS)
    fetch(bridgeApiUrl('/api/network-info'))
      .then((r) => (r.ok ? r.json() : null))
      .then((d: NetworkInfo | null) => setNet(d))
      .catch(() => {})
    return () => clearInterval(t)
  }, [])

  // The engineer at home reaches the hub over Tailscale; without it, the LAN.
  const host = net?.tailscale_ip || net?.ip
  const link = (token: string) => (host && net ? `http://${host}:${net.port}/?token=${token}` : '')
  const teamName = (id: string) => teams.find((t) => t.id === id)?.name ?? id

  return (
    <div style={{ flex: 1, minHeight: 0, overflow: 'auto', background: colors.bg, padding: 12 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 14, maxWidth: 980, margin: '0 auto' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontFamily: fonts.heading, fontSize: 26, color: colors.primary, letterSpacing: 1 }}>ACCESS</span>
          <span style={{ flex: 1 }} />
          <button onClick={onClose} style={btn(false)} title="Close">✕</button>
        </div>
        <span style={muted}>
          Each person gets a link of their own. <b style={{ color: colors.text }}>Engineers</b> run the race on every team
          (strategy, targets, Pin, asking the AI); <b style={{ color: colors.text }}>guests</b> only watch the teams you tick.
          Settings, teams, drivers and people stay on this PC. The hub link (🔗) is an engineer's link too.
          {net && !net.tailscale_ip ? ' Tailscale is not running on this PC: the links below work on this network only.' : ''}
        </span>
        {error ? <span style={{ color: colors.danger, fontFamily: fonts.body, fontSize: 14 }}>{error}</span> : null}

        <AddPerson teams={teams} />

        <Section title="People">
          {people === null ? <span style={muted}>Loading…</span>
            : people.length === 0 ? <span style={muted}>Nobody yet. Add a person above, then send them their link.</span>
              : people.map((p) => (
                <PersonRow key={p.id} p={p} teams={teams} link={link(p.token)} online={online.filter((o) => o.user_id === p.id)} teamName={teamName} />
              ))}
        </Section>

        <Section title="Connected now">
          {online.length === 0 ? <span style={muted}>Nobody.</span> : (
            <table style={{ borderCollapse: 'collapse', fontFamily: fonts.body, fontSize: 14, color: colors.text }}>
              <thead>
                <tr>{['Who', 'Access', 'Team', 'From', 'For'].map((h) => <th key={h} style={th}>{h}</th>)}</tr>
              </thead>
              <tbody>
                {online.map((o, i) => (
                  <tr key={i}>
                    <td style={td}>{o.name}</td>
                    <td style={td}>{ROLE_TEXT[o.role] ?? o.role}</td>
                    <td style={td}>{teamName(o.team)}</td>
                    <td style={{ ...td, fontFamily: fonts.mono, fontSize: 12 }}>{o.address}</td>
                    <td style={{ ...td, fontFamily: fonts.mono, fontSize: 12 }}>{fmtSince(o.connected_s)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Section>
      </div>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: 1 }}>{title}</span>
      {children}
    </div>
  )
}

/** Tick boxes for the teams a guest may watch. */
function TeamPicks({ teams, value, onChange }: { teams: TeamInfo[]; value: string[]; onChange: (v: string[]) => void }) {
  return (
    <span style={{ display: 'flex', gap: 10, flexWrap: 'wrap', alignItems: 'center' }}>
      {teams.map((t) => (
        <label key={t.id} style={{ display: 'flex', alignItems: 'center', gap: 4, fontFamily: fonts.body, fontSize: 14, color: colors.text, cursor: 'pointer' }}>
          <input
            type="checkbox"
            checked={value.includes(t.id)}
            onChange={(e) => onChange(e.target.checked ? [...value, t.id] : value.filter((x) => x !== t.id))}
          />
          <span style={{ width: 8, height: 8, borderRadius: '50%', background: t.color }} />
          {t.name}
        </label>
      ))}
    </span>
  )
}

function AddPerson({ teams }: { teams: TeamInfo[] }) {
  const [name, setName] = useState('')
  const [role, setRole] = useState<'engineer' | 'guest'>('engineer')
  const [picked, setPicked] = useState<string[]>([])
  const add = () => {
    if (!name.trim()) return
    sendBridgeCommand({ command: 'access_add', name: name.trim(), role, teams: role === 'guest' ? picked : [] })
    setName('')
    setPicked([])
  }
  return (
    <div style={{ ...card, flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 10 }}>
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') add() }}
        placeholder="Name"
        style={{ ...input, width: 180 }}
      />
      <RolePick value={role} onChange={setRole} />
      {role === 'guest' ? <TeamPicks teams={teams} value={picked} onChange={setPicked} /> : null}
      <button onClick={add} disabled={!name.trim()} style={{ ...btn(!!name.trim()), opacity: name.trim() ? 1 : 0.5 }}>Add</button>
    </div>
  )
}

function RolePick({ value, onChange }: { value: 'engineer' | 'guest'; onChange: (r: 'engineer' | 'guest') => void }) {
  return (
    <span style={{ display: 'flex' }}>
      {(['engineer', 'guest'] as const).map((r) => (
        <button key={r} onClick={() => onChange(r)} style={btn(value === r)}>{r === 'engineer' ? 'Engineer' : 'Guest'}</button>
      ))}
    </span>
  )
}

function PersonRow({ p, teams, link, online, teamName }: {
  p: PersonInfo; teams: TeamInfo[]; link: string; online: OnlineInfo[]; teamName: (id: string) => string
}) {
  const update = (change: { role?: string; teams?: string[] }) => sendBridgeCommand({ command: 'access_update', id: p.id, ...change })
  const regenerate = () => {
    if (window.confirm(`Give ${p.name} a new link? The current one stops working at once.`)) sendBridgeCommand({ command: 'access_regenerate', id: p.id })
  }
  const remove = () => {
    if (window.confirm(`Remove ${p.name}? They are disconnected at once and their link stops working.`)) sendBridgeCommand({ command: 'access_remove', id: p.id })
  }
  return (
    <div style={card}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
        <span style={{ width: 9, height: 9, borderRadius: '50%', background: online.length ? colors.success : colors.border, flexShrink: 0 }} />
        <span style={{ fontFamily: fonts.body, fontSize: 17, color: colors.text }}>{p.name}</span>
        <span style={{ fontFamily: fonts.body, fontSize: 12, color: online.length ? colors.success : colors.textMuted }}>
          {online.length ? `online · ${online.map((o) => `${teamName(o.team)}, ${fmtSince(o.connected_s)}`).join(' · ')}` : 'not connected'}
        </span>
        <span style={{ flex: 1 }} />
        <RolePick value={p.role} onChange={(role) => update({ role })} />
        <button onClick={regenerate} style={btn(false)} title="The current link stops working">New link</button>
        <button onClick={remove} style={{ ...btn(false), color: colors.danger }}>Remove</button>
      </div>
      {p.role === 'guest' ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
          <span style={muted}>Watches:</span>
          <TeamPicks teams={teams} value={p.teams} onChange={(t) => update({ teams: t })} />
          {p.teams.length === 0 ? <span style={{ ...muted, color: colors.accent }}>no team: they cannot connect</span> : null}
        </div>
      ) : null}
      {link ? <LinkRow label={`${p.name}'s link`} url={link} /> : null}
    </div>
  )
}

const muted: React.CSSProperties = { fontFamily: fonts.body, fontSize: 14, color: colors.textMuted, lineHeight: 1.4 }
const card: React.CSSProperties = {
  display: 'flex', flexDirection: 'column', gap: 8, padding: 10, background: colors.bgWidget,
  border: `1px solid ${colors.border}`, borderRadius: 4,
}
const input: React.CSSProperties = {
  background: colors.bg, border: `1px solid ${colors.border}`, color: colors.text, fontFamily: fonts.body,
  fontSize: 15, padding: '5px 8px', borderRadius: 3,
}
const th: React.CSSProperties = { textAlign: 'left', padding: '4px 12px 4px 0', color: colors.textMuted, fontWeight: 400, fontSize: 12, textTransform: 'uppercase', letterSpacing: 1 }
const td: React.CSSProperties = { padding: '4px 12px 4px 0', borderTop: `1px solid ${colors.border}` }

function btn(active: boolean): React.CSSProperties {
  return {
    background: active ? colors.primary : colors.bgWidget, color: active ? '#000' : colors.text,
    border: `1px solid ${active ? colors.primary : colors.border}`, borderRadius: 3, fontFamily: fonts.body,
    fontSize: 13, padding: '4px 10px', cursor: 'pointer', whiteSpace: 'nowrap',
  }
}
