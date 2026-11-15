import { useCallback, useEffect, useRef, useState } from 'react'
import { useLayoutStore, LAYOUT_PRESETS } from '../stores/layoutStore'
import { useTelemetryStore } from '../stores/telemetryStore'
import { useSettingsStore } from '../stores/settingsStore'
import { WIDGET_REGISTRY } from './widgetRegistry'
import { colors, fonts } from '../styles/theme'
import { bridgeApiUrl } from '../utils/bridge'
import { isFullscreen, keepAwakeActive, setKeepAwake, toggleFullscreen as toggleFs } from '../utils/device'

interface NetworkInfo {
  ip: string
  port: number
  local?: boolean
  tailscale_ip?: string | null
  token?: string
}

/**
 * On the racing PC: a "Remote link" button that shows the links to send to
 * the engineer (address + access token in one URL). The bridge only reveals
 * the token to its own machine, so on any other device this is just the
 * address, as before.
 */
function NetworkBadge() {
  const wsHost = useSettingsStore((s) => s.wsHost)
  const wsPort = useSettingsStore((s) => s.wsPort)
  const bridgeToken = useSettingsStore((s) => s.bridgeToken)
  const [info, setInfo] = useState<NetworkInfo | null>(null)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    fetch(bridgeApiUrl('/api/network-info'))
      .then((r) => (r.ok ? r.json() : null))
      .then((data: NetworkInfo | null) => setInfo(data))
      .catch(() => setInfo(null))
  }, [wsHost, wsPort, bridgeToken, open])

  if (!info) return null

  if (!info.local || !info.token) {
    return (
      <span
        title="Network address of the bridge"
        style={{
          fontFamily: fonts.body,
          fontSize: 13,
          color: colors.textMuted,
          letterSpacing: 0.3,
          userSelect: 'text',
          cursor: 'text',
          whiteSpace: 'nowrap',
        }}
      >
        {info.ip}:{info.port}
      </span>
    )
  }

  const link = (host: string) => `http://${host}:${info.port}/?token=${info.token}`

  return (
    <div style={{ position: 'relative' }}>
      <ToolbarBtn onClick={() => setOpen((o) => !o)} title="Link for your engineer or a tablet" active={open}>
        🔗
      </ToolbarBtn>
      {open && (
        <div style={{
          position: 'absolute',
          right: 0,
          top: '115%',
          zIndex: 1000,
          width: 'min(92vw, 460px)',
          background: colors.bgCard,
          border: `1px solid ${colors.border}`,
          borderRadius: 4,
          padding: 12,
          display: 'flex',
          flexDirection: 'column',
          gap: 10,
          boxShadow: '0 8px 24px rgba(0,0,0,0.6)',
          fontFamily: fonts.body,
          fontSize: 14,
          color: colors.textMuted,
        }}>
          <div style={{ color: colors.text, fontSize: 16 }}>Remote link</div>
          {info.tailscale_ip ? (
            <LinkRow label="Over Tailscale (engineer at home)" url={link(info.tailscale_ip)} />
          ) : (
            <div>
              Tailscale is not running on this PC. Install and sign in to Tailscale
              to get a link that works from outside your home network.
            </div>
          )}
          <LinkRow label="Same Wi-Fi / LAN (tablet next to you)" url={link(info.ip)} />
          <div style={{ fontSize: 12 }}>
            The link contains your access token. Send it only to people who should
            see your telemetry. Whoever opens it can watch, not control this PC.
          </div>
        </div>
      )}
    </div>
  )
}

function LinkRow({ label, url }: { label: string; url: string }) {
  const [copied, setCopied] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)

  const copy = useCallback(() => {
    const done = () => { setCopied(true); setTimeout(() => setCopied(false), 1500) }
    // The clipboard API needs a secure context; localhost is one. Fall back
    // to selecting the text so Ctrl+C works anywhere.
    if (navigator.clipboard?.writeText) {
      navigator.clipboard.writeText(url).then(done).catch(() => inputRef.current?.select())
    } else {
      inputRef.current?.select()
    }
  }, [url])

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
      <span style={{ fontSize: 12, textTransform: 'uppercase', letterSpacing: 1 }}>{label}</span>
      <div style={{ display: 'flex', gap: 6 }}>
        <input
          ref={inputRef}
          readOnly
          value={url}
          onFocus={(e) => e.currentTarget.select()}
          style={{
            flex: 1,
            minWidth: 0,
            background: colors.bgWidget,
            border: `1px solid ${colors.border}`,
            color: colors.text,
            fontFamily: fonts.mono,
            fontSize: 12,
            padding: '5px 7px',
            borderRadius: 3,
          }}
        />
        <button
          onClick={copy}
          style={{
            background: copied ? colors.success : colors.primary,
            color: '#000',
            border: 'none',
            borderRadius: 3,
            fontFamily: fonts.body,
            fontSize: 13,
            padding: '4px 12px',
            cursor: 'pointer',
            whiteSpace: 'nowrap',
          }}
        >
          {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
    </div>
  )
}

function UpdateBadge() {
  const versionInfo = useTelemetryStore((s) => s.versionInfo)
  if (!versionInfo?.update_available) return null

  return (
    <a
      href={versionInfo.download_url}
      target="_blank"
      rel="noreferrer"
      title={`Update available: v${versionInfo.latest_version}`}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 5,
        padding: '3px 8px',
        background: `${colors.accent}22`,
        border: `1px solid ${colors.accent}`,
        borderRadius: 3,
        color: colors.accent,
        fontFamily: fonts.body,
        fontSize: 14,
        textDecoration: 'none',
        whiteSpace: 'nowrap',
      }}
    >
      ↑ v{versionInfo.latest_version}
    </a>
  )
}

/** On a hub: who is on screen right now. Nothing on an ordinary bridge. */
function HubBadge() {
  const status = useTelemetryStore((s) => s.hub.status)
  const widgets = useLayoutStore((s) => s.widgets)
  const addWidget = useLayoutStore((s) => s.addWidget)
  if (!status) return null
  const hasPanel = widgets.some((w) => w.widgetType === 'TeamWidget')
  const active = status.sources.find((s) => s.id === status.active_id)
  const text = active
    ? `${active.label}${active.driving ? '' : ' (not driving)'}`
    : status.sources.length ? 'nobody connected' : 'no drivers added'
  return (
    <button
      onClick={() => { if (!hasPanel) addWidget('TeamWidget', registrySize('TeamWidget')) }}
      title={`${status.follow ? 'Hub pinned to this PC' : 'Hub following whoever is driving'}${hasPanel ? '' : ' — click to add the Team panel'}`}
      style={{
        fontFamily: fonts.body,
        fontSize: 15,
        padding: '3px 8px',
        borderRadius: 3,
        whiteSpace: 'nowrap',
        background: 'transparent',
        cursor: hasPanel ? 'default' : 'pointer',
        color: active?.driving ? colors.primary : colors.textMuted,
        border: `1px solid ${active?.driving ? colors.primary : colors.border}`,
      }}
    >
      👥 {text}{status.follow ? ' 📌' : ''}
    </button>
  )
}

/** Registry size of a widget type, in the shape the layout store wants. */
function registrySize(widgetType: string) {
  const meta = WIDGET_REGISTRY[widgetType]
  if (!meta) return undefined
  return { w: meta.defaultSize.w, h: meta.defaultSize.h, minW: meta.minSize.w, minH: meta.minSize.h }
}

function ConnectionBadge() {
  const status = useTelemetryStore((s) => s.connection.status)
  const gameConnected = useTelemetryStore((s) => s.connection.game_connected)
  const sessionActive = useTelemetryStore((s) => s.connection.session_active)

  const dotColor =
    status === 'connected' ? colors.success
    : status === 'reconnecting' ? colors.accent
    : colors.danger

  const label =
    status === 'connected' && gameConnected && sessionActive ? 'Live'
    : status === 'connected' && gameConnected ? 'Menu'
    : status === 'connected' ? 'Bridge'
    : status === 'reconnecting' ? '…'
    : status === 'unauthorized' ? 'Token?'
    : 'Off'

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
      <div style={{ width: 10, height: 10, borderRadius: '50%', background: dotColor, boxShadow: `0 0 5px ${dotColor}` }} />
      <span style={{ fontFamily: fonts.body, fontSize: 18, color: dotColor }}>{label}</span>
    </div>
  )
}

export default function Toolbar({ onOpenSettings, onOpenResults, onOpenFuel, onOpenEngineer, resultsOpen, fuelOpen, engineerOpen }: {
  onOpenSettings: () => void
  onOpenResults: () => void
  onOpenFuel: () => void
  onOpenEngineer: () => void
  resultsOpen: boolean
  fuelOpen: boolean
  engineerOpen: boolean
}) {
  const { activePreset, locked, setPreset, resetToDefault, toggleLock, addWidget } = useLayoutStore()
  const fullscreen = useSettingsStore((s) => s.fullscreen)

  const toggleFullscreen = useCallback(() => toggleFs(), [])
  const [fsOn, setFsOn] = useState(isFullscreen())
  useEffect(() => {
    const update = () => setFsOn(isFullscreen())
    document.addEventListener('fullscreenchange', update)
    document.addEventListener('webkitfullscreenchange', update)
    return () => {
      document.removeEventListener('fullscreenchange', update)
      document.removeEventListener('webkitfullscreenchange', update)
    }
  }, [])
  const [awake, setAwake] = useState(keepAwakeActive())
  const toggleAwake = useCallback(() => {
    setKeepAwake(!keepAwakeActive()).then(setAwake)
  }, [])

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      flexWrap: 'wrap',
      gap: 10,
      rowGap: 6,
      padding: '7px 14px',
      paddingTop: 'max(7px, env(safe-area-inset-top))',
      paddingLeft: 'max(14px, env(safe-area-inset-left))',
      paddingRight: 'max(14px, env(safe-area-inset-right))',
      background: colors.bgCard,
      borderBottom: `1px solid ${colors.border}`,
      flexShrink: 0,
      minHeight: 52,
    }}>
      {/* Title */}
      <span style={{
        fontFamily: fonts.heading,
        fontSize: 30,
        fontWeight: 700,
        color: colors.primary,
        letterSpacing: 2,
        lineHeight: 1,
        marginRight: 8,
      }}>
        LMU
      </span>

      <ConnectionBadge />
      <HubBadge />

      <div style={{ width: 1, height: 27, background: colors.border, margin: '0 4px' }} />

      {/* Preset selector */}
      <select
        value={activePreset}
        onChange={(e) => setPreset(e.target.value)}
        style={{
          background: colors.bgWidget,
          border: `1px solid ${colors.border}`,
          color: colors.text,
          fontFamily: fonts.body,
          fontSize: 18,
          padding: '4px 8px',
          borderRadius: 3,
          cursor: 'pointer',
        }}
      >
        {Object.keys(LAYOUT_PRESETS).map((name) => (
          <option key={name} value={name}>{name}</option>
        ))}
      </select>

      {/* Reset */}
      <ToolbarBtn onClick={resetToDefault} title="Reset layout">↺</ToolbarBtn>

      <div style={{ width: 1, height: 27, background: colors.border, margin: '0 4px' }} />

      {/* Add Widget */}
      {!locked && (
        <select
          value=""
          onChange={(e) => { if (e.target.value) addWidget(e.target.value, registrySize(e.target.value)) }}
          style={{
            background: colors.bgWidget,
            border: `1px solid ${colors.border}`,
            color: colors.textMuted,
            fontFamily: fonts.body,
            fontSize: 18,
            padding: '4px 8px',
            borderRadius: 3,
            cursor: 'pointer',
          }}
        >
          <option value="" disabled>+ Add widget</option>
          {Object.values(WIDGET_REGISTRY).map((meta) => (
            <option key={meta.id} value={meta.id}>{meta.name}</option>
          ))}
        </select>
      )}

      {/* Spacer */}
      <div style={{ flex: 1 }} />

      {/* Race Engineer */}
      <ToolbarBtn onClick={onOpenEngineer} title="Race Engineer" active={engineerOpen}>🎙</ToolbarBtn>

      {/* Fuel Calculator */}
      <ToolbarBtn onClick={onOpenFuel} title="Fuel Calculator" active={fuelOpen}>⛽</ToolbarBtn>

      {/* Post Race Results */}
      <ToolbarBtn onClick={onOpenResults} title="Post Race Results" active={resultsOpen}>📋</ToolbarBtn>

      {/* Lock */}
      <ToolbarBtn
        onClick={toggleLock}
        title={locked ? 'Unlock layout' : 'Lock layout'}
        active={locked}
      >
        {locked ? '🔒' : '🔓'}
      </ToolbarBtn>

      {/* Fullscreen */}
      <ToolbarBtn onClick={toggleFullscreen} title="Toggle fullscreen (F11)" active={fullscreen || fsOn}>⛶</ToolbarBtn>

      {/* Keep the tablet's screen on during a race */}
      <ToolbarBtn onClick={toggleAwake} title={awake ? 'Screen kept on — tap to allow sleep' : 'Keep the screen on'} active={awake}>☀</ToolbarBtn>

      {/* Settings */}
      <ToolbarBtn onClick={onOpenSettings} title="Settings">⚙</ToolbarBtn>

      <div style={{ width: 1, height: 27, background: colors.border, margin: '0 2px' }} />

      {/* Remote link (on the racing PC) or bridge address (elsewhere) */}
      <NetworkBadge />

      <div style={{ width: 1, height: 27, background: colors.border, margin: '0 2px' }} />

      {/* Version + update badge */}
      <span style={{
        fontFamily: fonts.body,
        fontSize: 15,
        color: colors.textMuted,
        letterSpacing: 0.5,
        userSelect: 'none',
      }}>
        v{__APP_VERSION__}
      </span>
      <UpdateBadge />
    </div>
  )
}

function ToolbarBtn({
  onClick,
  children,
  title,
  active,
}: {
  onClick: () => void
  children: React.ReactNode
  title?: string
  active?: boolean
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      style={{
        background: active ? `${colors.primary}22` : colors.bgWidget,
        border: `1px solid ${active ? colors.primary : colors.border}`,
        color: active ? colors.primary : colors.textMuted,
        fontFamily: fonts.body,
        fontSize: 20,
        padding: '4px 10px',
        borderRadius: 3,
        cursor: 'pointer',
        lineHeight: 1.2,
      }}
    >
      {children}
    </button>
  )
}
