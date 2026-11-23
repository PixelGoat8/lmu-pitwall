import { useEffect, useState } from 'react'
import { bridgeApiUrl } from '../utils/bridge'

export type Role = 'admin' | 'engineer' | 'guest'

/** Who this page is to the bridge or hub it is connected to. */
export interface Access {
  /** Known once the bridge has answered; until then nothing is allowed. */
  known: boolean
  /** Open on the bridge's (or hub's) own PC. */
  local: boolean
  role: Role
  /** A person's name, or what the access is ("hub link", "this PC"). */
  name: string
  hub: boolean
  /** May change the race: strategy, targets, pin a driver, ask the AI. */
  canChange: boolean
  /** May change the AI settings and budget (on a hub, only its own PC). */
  canAiSettings: boolean
}

const UNKNOWN: Access = { known: false, local: false, role: 'guest', name: '', hub: false, canChange: false, canAiSettings: false }

interface AuthCheck { ok?: boolean; local?: boolean; role?: Role; name?: string; hub?: boolean }

function fromCheck(d: AuthCheck | null): Access {
  if (!d?.ok) return UNKNOWN
  const local = !!d.local
  // A bridge older than roles: its own PC is the admin, a token an engineer.
  const role: Role = d.role ?? (local ? 'admin' : 'engineer')
  const hub = !!d.hub
  return {
    known: true,
    local,
    role,
    name: d.name ?? '',
    hub,
    canChange: role !== 'guest',
    canAiSettings: role === 'admin' || (!hub && role === 'engineer'),
  }
}

// One request per address, shared by every widget that asks.
const pending = new Map<string, Promise<Access>>()

function fetchAccess(): Promise<Access> {
  const url = bridgeApiUrl('/api/auth-check')
  let p = pending.get(url)
  if (!p) {
    p = fetch(url)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: AuthCheck | null) => fromCheck(d))
      .catch(() => UNKNOWN)
    p.then((a) => { if (!a.known) pending.delete(url) })
    pending.set(url, p)
  }
  return p
}

/**
 * What this page may do. The bridge (or hub) refuses anything else whatever
 * the page shows, so this only decides which controls appear.
 */
export function useAccess(): Access {
  const [access, setAccess] = useState<Access>(UNKNOWN)
  useEffect(() => {
    let alive = true
    fetchAccess().then((a) => { if (alive) setAccess(a) })
    return () => { alive = false }
  }, [])
  return access
}
