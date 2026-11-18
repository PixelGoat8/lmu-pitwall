import { useEffect, useState } from 'react'
import { useWebSocket } from './hooks/useWebSocket'
import { colors } from './styles/theme'
import Toolbar from './components/Toolbar'
import WidgetGrid from './components/WidgetGrid'
import Settings from './components/Settings'
import PostRaceResults from './components/PostRaceResults'
import FuelCalculator from './components/FuelCalculator'
import HistoryPanel from './components/HistoryPanel'
import { useSettingsStore } from './stores/settingsStore'

export default function App() {
  useWebSocket()
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [resultsOpen, setResultsOpen] = useState(false)
  const [fuelOpen, setFuelOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const updateSettings = useSettingsStore((s) => s.update)

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
        onOpenResults={() => { setFuelOpen(false); setHistoryOpen(false); setResultsOpen(v => !v) }}
        onOpenFuel={() => { setResultsOpen(false); setHistoryOpen(false); setFuelOpen(v => !v) }}
        onOpenHistory={() => { setResultsOpen(false); setFuelOpen(false); setHistoryOpen(v => !v) }}
        historyOpen={historyOpen}
        resultsOpen={resultsOpen}
        fuelOpen={fuelOpen}
      />
      {historyOpen
        ? <HistoryPanel onClose={() => setHistoryOpen(false)} />
        : resultsOpen
        ? <PostRaceResults onClose={() => setResultsOpen(false)} />
        : fuelOpen
          ? <FuelCalculator onClose={() => setFuelOpen(false)} />
          : <WidgetGrid />
      }
      <Settings open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  )
}
