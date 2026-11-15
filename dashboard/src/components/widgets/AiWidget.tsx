import { useState } from 'react'
import { useTelemetryStore } from '../../stores/telemetryStore'
import { sendBridgeCommand } from '../../hooks/useWebSocket'
import { colors, fonts } from '../../styles/theme'
import type { AiComment, AiConfig } from '../../types/ai'

const TRIGGER: Record<string, string> = {
  lap: 'lap', question: 'question', fcy: 'full course yellow', fcy_end: 'green again', pit_out: 'after the stop',
  penalty: 'penalty', driver: 'driver change', impact: 'impact', position: 'position change',
}

/**
 * AI engineer comments: short notes for the engineer (not the driver), made
 * from the numbers the strategy software computed. Any comment with a number
 * not in those data is withheld, and the widget says so.
 */
export default function AiWidget() {
  const feed = useTelemetryStore((s) => s.aiFeed)
  const [question, setQuestion] = useState('')
  const [settings, setSettings] = useState(false)

  if (!feed) {
    return <div style={{ ...frame, justifyContent: 'center', alignItems: 'center', color: colors.textMuted, fontFamily: fonts.body, fontSize: 15 }}>AI engineer — no connection to the bridge</div>
  }
  const st = feed.status
  const cfg = st.config
  const ask = () => {
    sendBridgeCommand({ command: 'ai_ask', question: question.trim() || null })
    setQuestion('')
  }
  const toggle = () => cfg && sendBridgeCommand({ command: 'ai_config', config: { ...cfg, enabled: !cfg.enabled } })
  const comments = [...feed.comments].reverse()

  return (
    <div style={frame}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <button onClick={toggle} disabled={!st.key_present} style={pill(st.enabled)}>{st.enabled ? 'AI on' : 'AI off'}</button>
        <span style={{ fontFamily: fonts.mono, fontSize: 12, color: colors.textMuted }}>
          today ${st.cost_today_usd.toFixed(3)}{cfg ? ` / $${cfg.daily_budget_usd.toFixed(2)}` : ''} · {st.calls_today} comments
          {st.busy ? ' · thinking…' : ''}
        </span>
        <span style={{ flex: 1 }} />
        <button onClick={() => setSettings((o) => !o)} style={pill(settings)}>⚙</button>
      </div>
      {st.blocked ? <Note color={colors.accent}>{st.blocked}</Note> : null}
      {st.last_error ? <Note color={colors.danger}>{st.last_error}</Note> : null}
      {settings && cfg ? <Settings cfg={cfg} /> : null}

      <div style={{ display: 'flex', gap: 6 }}>
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') ask() }}
          placeholder="Ask (empty = comment now)"
          disabled={!st.enabled}
          style={{ ...input, flex: 1, minWidth: 0 }}
        />
        <button onClick={ask} disabled={!st.enabled} style={{ ...pill(true), opacity: st.enabled ? 1 : 0.4 }}>Ask</button>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 6, overflow: 'auto', minHeight: 0 }}>
        {comments.length === 0 ? (
          <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.textMuted }}>No comments yet.</span>
        ) : comments.map((c) => <Comment key={c.id} c={c} />)}
      </div>
    </div>
  )
}

function Comment({ c }: { c: AiComment }) {
  const head = `L${c.lap} · ${TRIGGER[c.trigger] ?? c.trigger}`
  return (
    <div style={{ borderLeft: `3px solid ${c.withheld || c.refused ? colors.danger : colors.primary}`, padding: '2px 8px', display: 'flex', flexDirection: 'column', gap: 2 }}>
      <span style={{ fontFamily: fonts.mono, fontSize: 11, color: colors.textMuted }} title={`${c.model} · ${c.usage.input_tokens} in / ${c.usage.output_tokens} out tokens · ${c.latency_ms} ms`}>
        {head} · ${c.cost_usd.toFixed(4)}
      </span>
      {c.question ? <span style={{ fontFamily: fonts.body, fontSize: 13, color: colors.textMuted, fontStyle: 'italic' }}>“{c.question}”</span> : null}
      {c.refused ? (
        <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.danger }}>The model declined to answer.</span>
      ) : c.withheld ? (
        <span style={{ fontFamily: fonts.body, fontSize: 14, color: colors.danger }}>
          Withheld: it used numbers that are not in the data ({c.unverified.join(', ')}).
        </span>
      ) : (
        <>
          <span style={{ fontFamily: fonts.body, fontSize: 15, color: colors.text, whiteSpace: 'pre-wrap', lineHeight: 1.35 }}>{c.text}</span>
          {c.unverified.length ? (
            <span style={{ fontFamily: fonts.body, fontSize: 12, color: colors.danger }}>Not in the data: {c.unverified.join(', ')}</span>
          ) : null}
        </>
      )}
    </div>
  )
}

function Settings({ cfg }: { cfg: AiConfig }) {
  const [d, setD] = useState({
    model: cfg.model, effort: cfg.effort, every: String(cfg.every_laps), interval: String(cfg.min_interval_s),
    hour: String(cfg.max_calls_per_hour), budget: String(cfg.daily_budget_usd), language: cfg.language,
    events: cfg.on_events, unverified: cfg.show_unverified,
  })
  const n = (s: string, f: number) => (s.trim() === '' || isNaN(Number(s)) ? f : Number(s))
  const apply = () => sendBridgeCommand({
    command: 'ai_config',
    config: {
      ...cfg, model: d.model.trim(), effort: d.effort, every_laps: Math.round(n(d.every, 2)), min_interval_s: n(d.interval, 60),
      max_calls_per_hour: Math.round(n(d.hour, 40)), daily_budget_usd: n(d.budget, 3), language: d.language,
      on_events: d.events, show_unverified: d.unverified,
    } satisfies AiConfig,
  })
  const field = (label: string, key: 'model' | 'every' | 'interval' | 'hour' | 'budget' | 'language', w = 90) => (
    <label style={lbl}>{label}<input value={d[key]} onChange={(e) => setD({ ...d, [key]: e.target.value })} style={{ ...input, width: w }} /></label>
  )
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, alignItems: 'flex-end', padding: 8, background: colors.bgWidget, border: `1px solid ${colors.border}`, borderRadius: 4 }}>
      {field('Model', 'model', 150)}
      <label style={lbl}>Effort
        <select value={d.effort} onChange={(e) => setD({ ...d, effort: e.target.value })} style={{ ...input, width: 90 }}>
          {['low', 'medium', 'high'].map((x) => <option key={x}>{x}</option>)}
        </select>
      </label>
      {field('Every N laps', 'every', 60)}
      {field('Min interval (s)', 'interval', 70)}
      {field('Max per hour', 'hour', 60)}
      {field('Daily budget ($)', 'budget', 70)}
      {field('Language', 'language', 90)}
      <label style={{ ...lbl, flexDirection: 'row', alignItems: 'center', gap: 6 }}>
        <input type="checkbox" checked={d.events} onChange={(e) => setD({ ...d, events: e.target.checked })} /> on race events
      </label>
      <label style={{ ...lbl, flexDirection: 'row', alignItems: 'center', gap: 6 }} title="Show comments that failed the number check, flagged, instead of hiding them">
        <input type="checkbox" checked={d.unverified} onChange={(e) => setD({ ...d, unverified: e.target.checked })} /> show unverified
      </label>
      <button onClick={apply} style={pill(true)}>Apply</button>
    </div>
  )
}

function Note({ color, children }: { color: string; children: React.ReactNode }) {
  return <span style={{ fontFamily: fonts.body, fontSize: 13, color }}>{children}</span>
}

const frame: React.CSSProperties = { width: '100%', height: '100%', display: 'flex', flexDirection: 'column', gap: 8, padding: '6px 10px', boxSizing: 'border-box', overflow: 'hidden' }
const lbl: React.CSSProperties = { display: 'flex', flexDirection: 'column', gap: 2, fontFamily: fonts.body, fontSize: 12, color: colors.textMuted }
const input: React.CSSProperties = { background: colors.bgCard, border: `1px solid ${colors.border}`, color: colors.text, fontFamily: fonts.body, fontSize: 14, padding: '4px 6px', borderRadius: 3 }

function pill(active: boolean): React.CSSProperties {
  return {
    background: active ? colors.primary : colors.bgWidget,
    color: active ? '#000' : colors.textMuted,
    border: `1px solid ${active ? colors.primary : colors.border}`,
    borderRadius: 3, fontFamily: fonts.body, fontSize: 13, padding: '3px 10px', cursor: 'pointer', whiteSpace: 'nowrap',
  }
}
