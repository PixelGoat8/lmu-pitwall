import { useEffect, useState } from 'react'
import { useWebSocket } from './hooks/useWebSocket'
import { colors, fonts } from './styles/theme'
import Toolbar from './components/Toolbar'
import WidgetGrid from './components/WidgetGrid'
import Settings from './components/Settings'
import PostRaceResults from './components/PostRaceResults'
import FuelCalculator from './components/FuelCalculator'
import HistoryPanel from './components/HistoryPanel'
import AccessPanel from './components/AccessPanel'
import { useSettingsStore } from './stores/settingsStore'
import { useTeamStore } from './stores/teamStore'
import { useTelemetryStore } from './stores/telemetryStore'

export default function App() {
  useWebSocket()
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [resultsOpen, setResultsOpen] = useState(false)
  const [fuelOpen, setFuelOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [accessOpen, setAccessOpen] = useState(false)
  const updateSettings = useSettingsStore((s) => s.update)
  // A new team means a new car: widgets start over instead of carrying what
  // they remembered about the previous one.
  const team = useTeamStore((s) => s.team)
  const teamName = useTeamStore((s) => (s.teams.length > 1 ? s.teams.find((t) => t.id === s.current)?.name : undefined))
  useEffect(() => {
    document.title = teamName ? `${teamName} · LMU Pitwall` : 'LMU Pitwall'
  }, [teamName])

  // Restore fullscreen on mount
  useEffect(() => {
    if (useSettingsStore.getState().fullscreen) {
      document.documentElement.requestFullscreen().catch(() => {})
    }
  }, [])

  // Sync fullscreen state to store on any change
  useEffect(() => {
    const handler = () => {
      updateSettings({ fullscreen: !!document.fullscreenElement })
    }
    document.addEventListener('fullscreenchange', handler)
    return () => document.removeEventListener('fullscreenchange', handler)
  }, [updateSettings])

  // F11 fullscreen shortcut
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'F11') {
        e.preventDefault()
        if (!document.fullscreenElement) {
          document.documentElement.requestFullscreen().catch(() => {})
        } else {
          document.exitFullscreen().catch(() => {})
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  return (
    <div style={{
      width: '100vw',
      height: '100vh',
      background: colors.bg,
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <Toolbar
        onOpenSettings={() => setSettingsOpen(true)}
        onOpenResults={() => { setFuelOpen(false); setHistoryOpen(false); setAccessOpen(false); setResultsOpen(v => !v) }}
        onOpenFuel={() => { setResultsOpen(false); setHistoryOpen(false); setAccessOpen(false); setFuelOpen(v => !v) }}
        onOpenHistory={() => { setResultsOpen(false); setFuelOpen(false); setAccessOpen(false); setHistoryOpen(v => !v) }}
        onOpenAccess={() => { setResultsOpen(false); setFuelOpen(false); setHistoryOpen(false); setAccessOpen(v => !v) }}
        accessOpen={accessOpen}
        historyOpen={historyOpen}
        resultsOpen={resultsOpen}
        fuelOpen={fuelOpen}
      />
      {accessOpen
        ? <AccessPanel onClose={() => setAccessOpen(false)} />
        : historyOpen
        ? <HistoryPanel key={team} onClose={() => setHistoryOpen(false)} />
        : resultsOpen
        ? <PostRaceResults onClose={() => setResultsOpen(false)} />
        : fuelOpen
          ? <FuelCalculator onClose={() => setFuelOpen(false)} />
          : <WidgetGrid key={team} />
      }
      <Settings open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      <RefusalNotice />
    </div>
  )
}

/**
 * Why the bridge did not do what was just asked (a guest changing the
 * strategy, an engineer changing the hub's AI settings), for a few seconds.
 */
function RefusalNotice() {
  const refusal = useTelemetryStore((s) => s.refusal)
  // The refusal whose time is up, by when it arrived.
  const [expired, setExpired] = useState<number | null>(null)
  useEffect(() => {
    if (!refusal) return
    const t = setTimeout(() => setExpired(refusal.at), REFUSAL_MS)
    return () => clearTimeout(t)
  }, [refusal])
  if (!refusal || expired === refusal.at) return null
  return (
    <div role="status" style={{
      position: 'fixed', left: '50%', bottom: 24, transform: 'translateX(-50%)', maxWidth: 'calc(100vw - 32px)',
      background: colors.bgWidget, border: `1px solid ${colors.accent}`, color: colors.text, borderRadius: 4,
      padding: '8px 14px', fontFamily: fonts.body, fontSize: 15, zIndex: 1000, boxShadow: '0 4px 16px rgba(0,0,0,0.4)',
    }}>
      {refusal.message}
    </div>
  )
}

const REFUSAL_MS = 6000
