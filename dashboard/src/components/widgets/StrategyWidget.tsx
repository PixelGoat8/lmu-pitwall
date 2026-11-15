import { useState } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { sendBridgeCommand } from '../../hooks/useWebSocket'
import { colors, fonts } from '../../styles/theme'
import { fmtLap } from '../../utils/lapFormat'
import type { EnergyPlan, StrategyConfig, StrategyUpdate, WhatIf } from '../../types/strategy'

// Every figure on this widget comes from the bridge's strategy engine; the
// widget only formats. Missing data shows as "—" and the engine's notes say why.

const dash = '—'

function fmtNum(v: number | null | undefined, digits = 1): string {
  return v === null || v === undefined || !isFinite(v) ? dash : v.toFixed(digits)
}

/** An energy amount in its unit: litres, or a fraction shown as percent. */
function fmtEnergy(v: number | null | undefined, unit: string, digits = 1): string {
  if (v === null || v === undefined || !isFinite(v)) return dash
  return unit === '%' ? `${(v * 100).toFixed(digits)}%` : `${v.toFixed(digits)} L`
}

function fmtDuration(s: number | null | undefined): string {
  if (s === null || s === undefined || !isFinite(s)) return dash
  const t = Math.max(0, Math.round(s))
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  const sec = t % 60
  return h > 0 ? `${h}h${String(m).padStart(2, '0')}` : `${m}:${String(sec).padStart(2, '0')}`
}

/** ↑ / ↓ with the change per lap, coloured: for consumption, rising is bad. */
function Trend({ v, unit, risingIsBad = true }: { v: number | null; unit: string; risingIsBad?: boolean }) {
  if (v === null || !isFinite(v)) return null
  const small = unit === '%' ? Math.abs(v) < 0.0005 : unit === 's' ? Math.abs(v) < 0.01 : Math.abs(v) < 0.01
  if (small) return <span style={{ color: colors.textMuted }}> → steady</span>
  const bad = risingIsBad ? v > 0 : v < 0
  const text = unit === '%' ? `${(Math.abs(v) * 100).toFixed(2)}%` : unit === 's' ? `${Math.abs(v).toFixed(3)} s` : `${Math.abs(v).toFixed(2)} L`
  return <span style={{ color: bad ? colors.danger : colors.success }}> {v > 0 ? '↑' : '↓'} {text}/lap</span>
}

export default function StrategyWidget() {
  const s = useTelemetryStore((st) => st.strategy)
  const [showSettings, setShowSettings] = useState(false)

  if (!s) {
    return (
      <div style={{ ...frame, justifyContent: 'center', alignItems: 'center', color: colors.textMuted, fontFamily: fonts.body, fontSize: 15 }}>
        Strategy — waiting for a session
      </div>
    )
  }

  return (
    <div style={frame}>
      <Header s={s} onSettings={() => setShowSettings((o) => !o)} settingsOpen={showSettings} />
      {showSettings ? <Settings cfg={s.config} /> : null}
      <Energy s={s} />
      <Plan s={s} />
      {s.what_if.length > 0 ? <WhatIfTable rows={s.what_if} s={s} /> : null}
      <Tyres s={s} />
      <PitAndWeather s={s} />
      {s.notes.length > 0 ? (
        <ul style={{ margin: 0, paddingLeft: 16, fontFamily: fonts.body, fontSize: 12, color: colors.textMuted, lineHeight: 1.35 }}>
          {s.notes.map((n) => <li key={n}>{n}</li>)}
        </ul>
      ) : null}
    </div>
  )
}

// ---------------------------------------------------------------------------

function Header({ s, onSettings, settingsOpen }: { s: StrategyUpdate; onSettings: () => void; settingsOpen: boolean }) {
  const h = s.horizon
  return (
    <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, flexWrap: 'wrap' }}>
      <span style={{ fontFamily: fonts.mono, fontSize: 24, fontWeight: 700, color: colors.text, lineHeight: 1 }}>
        {h ? h.laps_to_go.toFixed(1) : dash}
      </span>
      <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }}>
        laps to go{h?.basis === 'time' && h.time_remaining_s !== null ? ` · ${fmtDuration(h.time_remaining_s)} left` : ''}
        {s.pace.ref_lap_s ? ` · ref lap ${fmtLap(s.pace.ref_lap_s)}` : ''}
        {s.stint.elapsed_s !== null ? ` · stint ${fmtDuration(s.stint.elapsed_s)}${s.stint.limit_s ? ` / ${fmtDuration(s.stint.limit_s)}` : ''}` : ''}
      </span>
      <span style={{ flex: 1 }} />
      <button onClick={onSettings} style={smallButton(settingsOpen)} title="Strategy settings">⚙ settings</button>
    </div>
  )
}

const MAX_SAVING = 0.25

/** Where the consumption the plan uses comes from, in a few words. */
function rateBasis(e: EnergyPlan): string {
  const r = e.rate
  const laps = `${r.laps} lap${r.laps === 1 ? '' : 's'}`
  if (r.basis === 'lico') return `since LiCo changed, ${laps}`
  if (r.basis === 'previous stint') return 'previous stint'
  return `this stint, ${laps}`
}

function EnergyColumn({ title, e, kind, cfg }: { title: string; e: EnergyPlan; kind: 've' | 'fuel'; cfg: StrategyConfig }) {
  const u = e.unit
  const digits = u === '%' ? 1 : 1
  const r = e.rate
  // A stop fewer that needs more than this saving is out of reach of
  // lift-and-coast: not worth a line.
  const sv = e.save_stop && e.save_stop.saving <= MAX_SAVING ? e.save_stop : null
  const t = e.target
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
      <span style={sectionTitle}>{title}</span>
      <span style={{ fontFamily: fonts.mono, fontSize: 18, color: colors.text }}>
        {fmtEnergy(e.now, u, digits)}
        <span style={{ fontSize: 13, color: colors.textMuted }}> · {fmtNum(e.laps_left, 1)} laps</span>
      </span>
      <span style={line} title={`Used by the plan: measured over ${rateBasis(e)}. A change of lift-and-coast restarts the measurement, so the plan follows it within two laps.`}>
        <b style={{ color: colors.text }}>{fmtEnergy(e.planning_per_lap, u, 2)}/lap</b>
        <span> · {rateBasis(e)}</span>
      </span>
      {r.coast_s !== null ? (
        <span style={line} title="Lift-and-coast: seconds per lap at speed (over 72 km/h) with neither throttle nor brake. Some of it is normal driving; what matters is the change.">
          LiCo {r.coast_s.toFixed(1)} s/lap
          {r.coast_before_s !== null ? (
            <span style={{ color: r.coast_s > r.coast_before_s ? colors.success : colors.accent }}>
              {' '}(was {r.coast_before_s.toFixed(1)} s{r.per_lap_before !== null ? ` at ${fmtEnergy(r.per_lap_before, u, 2)}/lap` : ''})
            </span>
          ) : null}
        </span>
      ) : null}
      <span style={line}>
        to finish {fmtEnergy(e.needed_to_finish, u)} ·{' '}
        <span style={{ color: (e.to_add ?? 0) > 0 ? colors.accent : colors.success }}>
          {e.to_add === null ? dash : e.to_add > 0 ? `add ${fmtEnergy(e.to_add, u)}` : 'enough on board'}
        </span>
      </span>
      <span style={line}>full tank: {fmtNum(e.stint_laps_max, 1)} laps · reserve {fmtEnergy(e.reserve, u)}</span>
      {sv ? (
        <span style={line} title="Highest consumption per lap that finishes with one stop fewer, and the coasting it takes (from how much each second of coasting has saved so far)">
          {sv.stops === 0 ? 'no stop' : `${sv.stops} stop${sv.stops > 1 ? 's' : ''}`} at ≤ {fmtEnergy(sv.per_lap, u, 2)}/lap
          <span style={{ color: sv.saving > 0.1 ? colors.danger : colors.accent }}> (−{(sv.saving * 100).toFixed(1)}%</span>
          {sv.extra_coast_s !== null ? <span style={{ color: sv.saving > 0.1 ? colors.danger : colors.accent }}>, ≈ +{sv.extra_coast_s.toFixed(1)} s LiCo/lap</span> : null}
          <span style={{ color: sv.saving > 0.1 ? colors.danger : colors.accent }}>)</span>
        </span>
      ) : null}
      <TargetLine e={e} t={t} kind={kind} cfg={cfg} />
    </div>
  )
}

/**
 * The engineer's target consumption: a field to set it, and once set, how far
 * off the driver is and what the target gives.
 */
function TargetLine({ e, t, kind, cfg }: { e: EnergyPlan; t: EnergyPlan['target']; kind: 've' | 'fuel'; cfg: StrategyConfig }) {
  const u = e.unit
  const pct = u === '%'
  const [draft, setDraft] = useState('')
  const send = (v: number | null) => {
    const config: StrategyConfig = kind === 've' ? { ...cfg, ve_target_per_lap: v } : { ...cfg, fuel_target_per_lap: v }
    sendBridgeCommand({ command: 'strategy_config', config })
    setDraft('')
  }
  const set = () => {
    const n = Number(draft.replace(',', '.'))
    if (draft.trim() === '' || !isFinite(n) || n <= 0) return
    send(pct ? n / 100 : n)
  }
  const over = t ? t.off_target : 0
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 2 }}>
      <span style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
        <span style={line}>target</span>
        <input
          value={draft}
          placeholder={t ? (pct ? (t.per_lap * 100).toFixed(2) : t.per_lap.toFixed(2)) : pct ? '%/lap' : 'L/lap'}
          inputMode="decimal"
          onChange={(ev) => setDraft(ev.target.value)}
          onKeyDown={(ev) => { if (ev.key === 'Enter') set() }}
          style={{ ...input, width: 70 }}
          title={pct ? 'Virtual energy per lap you ask the driver for, in % (e.g. 3.2)' : 'Fuel per lap you ask the driver for, in litres'}
        />
        <span style={line}>{pct ? '%/lap' : 'L/lap'}</span>
        <button onClick={set} disabled={draft.trim() === ''} style={{ ...smallButton(draft.trim() !== ''), opacity: draft.trim() === '' ? 0.5 : 1 }}>Set</button>
        {t ? <button onClick={() => send(null)} style={smallButton(false)}>Clear</button> : null}
      </span>
      {t ? (
        <>
          <span style={line}>
            now{' '}
            <b style={{ color: Math.abs(over) < 0.01 ? colors.success : over > 0 ? colors.danger : colors.success }}>
              {Math.abs(over) < 0.01 ? 'on target' : `${over > 0 ? '+' : '−'}${Math.abs(over * 100).toFixed(1)}% ${over > 0 ? 'over' : 'under'}`}
            </b>
            {t.extra_coast_s !== null && Math.abs(over) >= 0.01
              ? t.extra_coast_s > 0
                ? ` · needs ≈ +${t.extra_coast_s.toFixed(1)} s LiCo/lap`
                : ` · margin ≈ ${Math.abs(t.extra_coast_s).toFixed(1)} s LiCo/lap`
              : ''}
          </span>
          <span style={line}>
            at target: {fmtNum(t.laps_left, 1)} laps left · full tank {fmtNum(t.stint_laps_max, 1)} laps
            {t.stops !== null ? ` · ${t.stops === 0 ? 'no stop' : `${t.stops} stop${t.stops > 1 ? 's' : ''}`}` : ''}
          </span>
        </>
      ) : null}
    </div>
  )
}

function Energy({ s }: { s: StrategyUpdate }) {
  if (!s.fuel && !s.ve) return null
  return (
    <div style={{ display: 'grid', gridTemplateColumns: s.fuel && s.ve ? '1fr 1fr' : '1fr', gap: 12 }}>
      {s.fuel ? <EnergyColumn title="Fuel" e={s.fuel} kind="fuel" cfg={s.config} /> : null}
      {s.ve ? <EnergyColumn title="Virtual energy" e={s.ve} kind="ve" cfg={s.config} /> : null}
    </div>
  )
}

function Plan({ s }: { s: StrategyUpdate }) {
  const p = s.plan
  if (!p) return null
  const w = p.window
  const current = s.laps_completed + 1
  const limitText = p.limiting === 've' ? 'virtual energy' : p.limiting
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, flexWrap: 'wrap' }}>
        <span style={{ fontFamily: fonts.mono, fontSize: 20, fontWeight: 700, color: p.stops > 0 ? colors.primary : colors.success }}>
          {p.stops === 0 ? 'NO STOP' : `${p.stops} STOP${p.stops > 1 ? 'S' : ''}`}
        </span>
        <span style={line}>
          {w
            ? <>box between laps <b style={{ color: colors.text }}>{w.earliest_lap}</b> and <b style={{ color: colors.text }}>{w.latest_lap}</b></>
            : p.stops > 0 ? 'window unknown' : 'to the flag'}
          {' · '}stint limited by {limitText} ({fmtNum(p.stint_laps_max, 1)} laps)
        </span>
      </div>
      {w ? <WindowBar current={current} earliest={w.earliest_lap} latest={w.latest_lap} toGo={s.horizon?.laps_to_go ?? null} /> : null}
    </div>
  )
}

/** Laps from now to the flag, with the pit window marked. */
function WindowBar({ current, earliest, latest, toGo }: { current: number; earliest: number; latest: number; toGo: number | null }) {
  const end = toGo !== null ? current + Math.ceil(toGo) : latest + 5
  const span = Math.max(1, end - current)
  const pct = (lap: number) => `${Math.min(100, Math.max(0, ((lap - current) / span) * 100))}%`
  return (
    <div style={{ position: 'relative', height: 18, background: colors.bgWidget, borderRadius: 3, border: `1px solid ${colors.border}` }}>
      <div
        title={`Pit window: end of lap ${earliest} to end of lap ${latest}`}
        style={{ position: 'absolute', top: 0, bottom: 0, left: pct(earliest), width: `calc(${pct(latest + 1)} - ${pct(earliest)})`, background: `${colors.primary}55`, borderLeft: `2px solid ${colors.primary}`, borderRight: `2px solid ${colors.primary}` }}
      />
      <span style={{ position: 'absolute', left: 4, top: 1, fontFamily: fonts.mono, fontSize: 11, color: colors.textMuted }}>L{current}</span>
      <span style={{ position: 'absolute', right: 4, top: 1, fontFamily: fonts.mono, fontSize: 11, color: colors.textMuted }}>🏁 L{end}</span>
    </div>
  )
}

function WhatIfTable({ rows, s }: { rows: WhatIf[]; s: StrategyUpdate }) {
  const fuelU = s.fuel?.unit ?? 'L'
  const veU = s.ve?.unit ?? '%'
  const cell: React.CSSProperties = { padding: '2px 6px', textAlign: 'right', whiteSpace: 'nowrap' }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
      <span style={sectionTitle}>Box in … laps{s.config.change_tyres ? ' (with 4 new tyres)' : ''}</span>
      <div style={{ overflowX: 'auto' }}>
        <table style={{ width: '100%', borderCollapse: 'collapse', fontFamily: fonts.mono, fontSize: 13, color: colors.text }}>
          <thead>
            <tr style={{ color: colors.textMuted, fontFamily: fonts.body, fontSize: 12 }}>
              <th style={{ ...cell, textAlign: 'left' }}>In</th>
              <th style={cell}>Lap</th>
              {s.fuel ? <th style={cell}>Add fuel</th> : null}
              {s.ve ? <th style={cell}>Add VE</th> : null}
              <th style={cell}>Stops after</th>
              <th style={cell} title="Stationary time from the game's pit work times">Stopped</th>
              <th style={cell} title="Stationary time plus pit lane loss">Total</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.in_laps} style={{ borderTop: `1px solid ${colors.border}55`, color: !r.reachable ? colors.danger : r.stops_after > 0 ? colors.textMuted : colors.text }}>
                <td style={{ ...cell, textAlign: 'left' }}>{r.in_laps === 0 ? 'now' : `+${r.in_laps}`}</td>
                <td style={cell}>{r.pit_lap}</td>
                {s.fuel ? <td style={cell}>{fmtEnergy(r.fuel_add, fuelU)}</td> : null}
                {s.ve ? <td style={cell}>{fmtEnergy(r.ve_add, veU, 0)}</td> : null}
                <td style={cell}>{r.reachable ? r.stops_after : 'can\'t reach'}</td>
                <td style={cell}>{r.stationary_s !== null ? `${r.stationary_s.toFixed(1)} s` : dash}</td>
                <td style={cell}>{r.stop_cost_s !== null ? `${r.stop_cost_s.toFixed(1)} s` : dash}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

const CORNERS = ['FL', 'FR', 'RL', 'RR']

function Tyres({ s }: { s: StrategyUpdate }) {
  const t = s.tyres
  const c = s.tyre_choice
  const worst = t.wear_per_lap ? t.wear_per_lap.indexOf(Math.max(...t.wear_per_lap)) : -1
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
      <span style={sectionTitle}>Tyres</span>
      <span style={line}>
        {t.set_laps} laps on this set
        {worst >= 0 && t.wear_per_lap ? ` · ${CORNERS[worst]} wears fastest, ${(t.wear_per_lap[worst] * 100).toFixed(2)}%/lap` : ''}
        {t.laps_to_limit !== null ? ` · ${t.laps_to_limit.toFixed(0)} laps to ${(t.limit * 100).toFixed(0)}% tread` : ''}
      </span>
      <span style={line} title="Lap time change per lap on this set, from clean laps. It includes the car getting lighter as fuel burns, so the tyres alone lose a little more.">
        pace on this set{t.pace_trend_s_per_lap === null ? ` ${dash} (needs 4 clean laps)` : ''}
        <Trend v={t.pace_trend_s_per_lap} unit="s" />
      </span>
      {c ? (
        <span style={line}>
          new tyres at the stop: +{c.change_cost_s.toFixed(1)} s stopped
          {c.expected_gain_s !== null ? <>, ~{c.expected_gain_s.toFixed(1)} s back over the next stint</> : ', no measurable gain yet'}
          {c.must_change ? <b style={{ color: colors.danger }}> · must change: tread runs out next stint</b> : null}
        </span>
      ) : null}
    </div>
  )
}

function PitAndWeather({ s }: { s: StrategyUpdate }) {
  const pl = s.pit_loss
  const w = s.weather
  const est = pl.game_estimate
  const lapsText = (l: number | null) => (l !== null ? ` (~${Math.round(l)} laps)` : '')
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))', gap: 10 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
        <span style={sectionTitle}>Pit lane</span>
        <span style={line}>
          loss {pl.lane_loss_s !== null ? `${pl.lane_loss_s.toFixed(1)} s` : dash}
          {pl.source === 'measured' ? ` (measured, ${pl.samples} stop${pl.samples > 1 ? 's' : ''})` : pl.source === 'manual' ? ' (set by hand)' : ''}
        </span>
        {est ? (
          <span style={line} title="The game's own estimate for the pit menu as it is set now">
            {est.total > 0 ? `game estimate for the menu as set: ${est.total.toFixed(1)} s` : 'pit menu empty: nothing to compare with yet'}
          </span>
        ) : null}
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
        <span style={sectionTitle}>Weather</span>
        <span style={line}>
          {w.raining_now ? 'raining' : 'dry'} · track water {(w.wetness * 100).toFixed(0)}%
        </span>
        {w.rain_in_s !== null ? (
          <span style={{ ...line, color: colors.accent }}>
            rain likely in {fmtDuration(w.rain_in_s)}{lapsText(w.rain_in_laps)}, {Math.round((w.rain_chance ?? 0) * 100)}% chance
          </span>
        ) : null}
        {w.dry_in_s !== null ? (
          <span style={{ ...line, color: colors.success }}>dry again in {fmtDuration(w.dry_in_s)}{lapsText(w.dry_in_laps)}</span>
        ) : null}
        {w.rain_in_s === null && w.dry_in_s === null ? <span style={line}>no change in the forecast</span> : null}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------

function Settings({ cfg }: { cfg: StrategyConfig }) {
  const [draft, setDraft] = useState({
    reserve: String(cfg.reserve_laps),
    stint: cfg.max_stint_minutes !== null ? String(cfg.max_stint_minutes) : '',
    wear: String(Math.round(cfg.tyre_wear_limit * 100)),
    tyres: cfg.change_tyres,
    loss: cfg.pit_lane_loss_s !== null ? String(cfg.pit_lane_loss_s) : '',
    veTarget: cfg.ve_target_per_lap !== null ? String(+(cfg.ve_target_per_lap * 100).toFixed(3)) : '',
    fuelTarget: cfg.fuel_target_per_lap !== null ? String(cfg.fuel_target_per_lap) : '',
  })
  const num = (s: string) => (s.trim() === '' || isNaN(Number(s.replace(',', '.'))) ? null : Number(s.replace(',', '.')))
  const apply = () => {
    const config: StrategyConfig = {
      reserve_laps: num(draft.reserve) ?? 0.5,
      max_stint_minutes: num(draft.stint),
      tyre_wear_limit: (num(draft.wear) ?? 30) / 100,
      change_tyres: draft.tyres,
      pit_lane_loss_s: num(draft.loss),
      ve_target_per_lap: num(draft.veTarget) !== null ? (num(draft.veTarget) as number) / 100 : null,
      fuel_target_per_lap: num(draft.fuelTarget),
    }
    sendBridgeCommand({ command: 'strategy_config', config })
  }
  const field = (label: string, key: 'reserve' | 'stint' | 'wear' | 'loss' | 'veTarget' | 'fuelTarget', placeholder: string, title: string) => (
    <label style={{ display: 'flex', flexDirection: 'column', gap: 2, fontFamily: fonts.body, fontSize: 12, color: colors.textMuted }} title={title}>
      {label}
      <input
        value={draft[key]}
        placeholder={placeholder}
        inputMode="decimal"
        onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))}
        style={input}
      />
    </label>
  )
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 10, alignItems: 'flex-end', padding: 8, background: colors.bgWidget, borderRadius: 4, border: `1px solid ${colors.border}` }}>
      {field('Reserve (laps)', 'reserve', '0.5', 'Energy always kept in the tank')}
      {field('Max stint (min)', 'stint', 'none', 'Longest a driver may stay in the car; empty = no limit')}
      {field('Tyre limit (% tread)', 'wear', '30', 'Below this a tyre counts as finished')}
      {field('Pit lane loss (s)', 'loss', 'measured', 'Empty = use the measured figure for this track')}
      {field('VE target (%/lap)', 'veTarget', 'none', 'Virtual energy per lap you ask the driver for; empty = no target')}
      {field('Fuel target (L/lap)', 'fuelTarget', 'none', 'Fuel per lap you ask the driver for; empty = no target')}
      <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontFamily: fonts.body, fontSize: 13, color: colors.text }}>
        <input type="checkbox" checked={draft.tyres} onChange={(e) => setDraft((d) => ({ ...d, tyres: e.target.checked }))} />
        new tyres at the stop
      </label>
      <button onClick={apply} style={{ ...smallButton(true), padding: '5px 14px' }}>Apply</button>
    </div>
  )
}

// ---------------------------------------------------------------------------

const frame: React.CSSProperties = {
  width: '100%',
  height: '100%',
  display: 'flex',
  flexDirection: 'column',
  gap: 10,
  padding: '8px 10px',
  boxSizing: 'border-box',
  overflow: 'auto',
}

const sectionTitle: React.CSSProperties = {
  fontFamily: fonts.body,
  fontSize: 12,
  color: colors.textMuted,
  letterSpacing: 2,
  textTransform: 'uppercase',
}

const line: React.CSSProperties = {
  fontFamily: fonts.body,
  fontSize: 14,
  color: colors.textMuted,
  lineHeight: 1.3,
}

const input: React.CSSProperties = {
  width: 90,
  background: colors.bgCard,
  border: `1px solid ${colors.border}`,
  color: colors.text,
  fontFamily: fonts.mono,
  fontSize: 14,
  padding: '4px 6px',
  borderRadius: 3,
}

function smallButton(active: boolean): React.CSSProperties {
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
  }
}
