import { useEffect, useState } from 'react'
import { bridgeApiUrl } from '../utils/bridge'

/**
 * Is this page open on the bridge's (or hub's) own PC? Some actions are only
 * accepted from there, and the bridge refuses them from anywhere else, so the
 * controls are shown only where they work. False until the bridge has answered.
 */
export function useIsLocal(): boolean {
  const [local, setLocal] = useState(false)
  useEffect(() => {
    let alive = true
    fetch(bridgeApiUrl('/api/auth-check'))
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { local?: boolean } | null) => { if (alive) setLocal(!!d?.local) })
      .catch(() => {})
    return () => { alive = false }
  }, [])
  return local
}
