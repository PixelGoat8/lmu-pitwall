import { useSettingsStore } from '../stores/settingsStore'
import { teamQuery } from '../stores/teamStore'

function resolveTarget(): { host: string; port: string } {
  const { wsHost, wsPort } = useSettingsStore.getState()
  const host = wsHost.trim() || window.location.hostname
  const port = wsPort > 0 ? String(wsPort) : (window.location.port || '9000')
  return { host, port }
}

/** `?token=…` (or `&token=…`) when a token is stored, else nothing. */
function tokenQuery(prefix: '?' | '&'): string {
  const token = useSettingsStore.getState().bridgeToken.trim()
  return token ? `${prefix}token=${encodeURIComponent(token)}` : ''
}

/**
 * WebSocket URL of the bridge. Carries the access token as a query parameter,
 * because a browser cannot set headers on a WebSocket. A browser on the
 * racing PC itself needs no token; the bridge ignores it there. On a hub it
 * also names the team this page watches (see stores/teamStore).
 */
export function bridgeWsUrl(): string {
  const { host, port } = resolveTarget()
  const token = tokenQuery('?')
  const team = teamQuery()
  return `ws://${host}:${port}/${token}${team ? `${token ? '&' : '?'}${team}` : ''}`
}

export function bridgeHttpBase(): string {
  const { host, port } = resolveTarget()
  return `http://${host}:${port}`
}

/** URL of a bridge API endpoint, with the access token attached. */
export function bridgeApiUrl(path: string): string {
  return `${bridgeHttpBase()}${path}${tokenQuery(path.includes('?') ? '&' : '?')}`
}

export type BridgeAuth = 'ok' | 'unauthorized' | 'unreachable'

/**
 * Ask the bridge whether our token is accepted.
 *
 * A browser never learns why a WebSocket handshake failed, so a refused token
 * and an unreachable bridge look identical from the socket. This plain HTTP
 * call tells them apart, and the dashboard can say "wrong token" instead of
 * retrying forever in silence.
 */
export async function checkBridgeAuth(): Promise<BridgeAuth> {
  try {
    const res = await fetch(bridgeApiUrl('/api/auth-check'), { signal: AbortSignal.timeout(4000) })
    if (res.ok) return 'ok'
    return res.status === 401 ? 'unauthorized' : 'unreachable'
  } catch {
    return 'unreachable'
  }
}

/**
 * Pick up `?token=…` from the page URL, store it, and remove it from the
 * address bar.
 *
 * This is how a remote device is set up: the driver sends a link such as
 * `http://100.x.y.z:9000/?token=…`, the engineer opens it once, and the token
 * is remembered in this browser from then on. Removing it from the URL keeps
 * it out of screenshots and shared bookmarks.
 */
export function adoptTokenFromUrl(): void {
  const url = new URL(window.location.href)
  const token = url.searchParams.get('token')
  if (!token) return
  useSettingsStore.getState().update({ bridgeToken: token.trim() })
  url.searchParams.delete('token')
  window.history.replaceState(null, '', url.pathname + url.search + url.hash)
}
