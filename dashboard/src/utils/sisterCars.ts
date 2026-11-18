import { useMemo } from 'react'
import { useTeamStore } from '../stores/teamStore'

/** A car of ours entered by another team of the same hub. */
export interface SisterCar {
  team: string
  color: string
}

/**
 * The hub's other cars, by the name the game gives them ("Iron Lynx #63"),
 * which is how a vehicle in the standings is matched. Empty on an ordinary
 * bridge, with a single team, or while the other teams follow nobody. A car
 * in another race simply never matches.
 */
export function useSisterCars(): Map<string, SisterCar> {
  const teams = useTeamStore((s) => s.teams)
  const current = useTeamStore((s) => s.current)
  return useMemo(() => {
    const map = new Map<string, SisterCar>()
    for (const t of teams) {
      if (t.id !== current && t.car_name) map.set(t.car_name, { team: t.name, color: t.color })
    }
    return map
  }, [teams, current])
}
