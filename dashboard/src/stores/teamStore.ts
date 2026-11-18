import { create } from 'zustand'
import type { TeamInfo } from '../types/telemetry'
import { useTelemetryStore } from './telemetryStore'

/**
 * Which team of a hub this page watches.
 *
 * A hub runs several teams (one car each). The choice lives in the page
 * address, `?team=<id>`, not in the saved settings: two tabs can watch two
 * teams side by side, a reload keeps the team, and a link can point straight
 * at one. Without the parameter the hub sends its first team. An ordinary
 * bridge has one team and ignores all of this.
 */
interface TeamStore {
  /** The team asked for; empty = the hub's first. */
  team: string
  /** The hub's teams, every second; empty on an ordinary bridge. */
  teams: TeamInfo[]
  /** The team the hub says this page is on. */
  current: string | null
  setTeams: (teams: TeamInfo[], current: string) => void
  /** Switch team: the address bar follows, the connection is reopened. */
  setTeam: (id: string) => void
}

function teamFromUrl(): string {
  return new URLSearchParams(window.location.search).get('team')?.trim() ?? ''
}

function writeUrl(id: string) {
  const url = new URL(window.location.href)
  if (id) url.searchParams.set('team', id)
  else url.searchParams.delete('team')
  window.history.replaceState(null, '', url.pathname + url.search + url.hash)
}

export const useTeamStore = create<TeamStore>((set, get) => ({
  team: teamFromUrl(),
  teams: [],
  current: null,

  setTeams: (teams, current) => {
    // Sent every second, mostly unchanged: keep the same array then, so
    // nothing re-renders for nothing.
    const same = JSON.stringify(teams) === JSON.stringify(get().teams)
    if (!same || current !== get().current) set({ teams: same ? get().teams : teams, current })
    // Asked for a team the hub does not have (deleted, or a stale link): the
    // hub has put us on its first one. Say so in the address bar.
    const asked = get().team
    if (asked && !teams.some((t) => t.id === asked)) {
      set({ team: current })
      writeUrl(current)
    }
  },

  setTeam: (id) => {
    if (id === (get().team || get().current)) return
    writeUrl(id)
    // Nothing of the old car may linger on screen while the new one loads.
    useTelemetryStore.getState().resetForTeam()
    set({ team: id, current: null })
  },
}))

/** `team=<id>` for the hub's WebSocket, or nothing. */
export function teamQuery(): string {
  const team = useTeamStore.getState().team
  return team ? `team=${encodeURIComponent(team)}` : ''
}
