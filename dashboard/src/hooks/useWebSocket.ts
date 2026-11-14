import { useEffect, useRef, useCallback } from 'react'
import { decode } from '@msgpack/msgpack'
import { useTelemetryStore } from '../stores/telemetryStore'
import { useSettingsStore } from '../stores/settingsStore'
import { bridgeWsUrl, checkBridgeAuth } from '../utils/bridge'
import type { ServerMessage } from '../types/telemetry'

/**
 * - `connected`     socket open and the bridge is talking
 * - `disconnected`  just lost it; first retry pending
 * - `reconnecting`  retrying
 * - `unauthorized`  the bridge is reachable but refuses our token
 */
export type ConnectionStatus = 'connected' | 'disconnected' | 'reconnecting' | 'unauthorized'

// Reconnect fast and keep trying: the engineer's link runs over the internet
// and a tablet's Wi-Fi drops for seconds at a time. A long backoff would turn
// a two-second blip into half a minute without data.
const BASE_DELAY_MS = 500
const MAX_DELAY_MS = 5_000
const JITTER_MS = 300

// The bridge sends a Heartbeat every 2 s even with no session running, so
// this long without any message means the link is dead even though the
// socket still says "open" — which is what happens when a network path
// vanishes without closing the connection.
const STALE_AFTER_MS = 7_000
// A connection attempt that has not completed in this time is abandoned and
// retried. Left alone, a browser waits minutes on a path that swallows packets.
const CONNECT_TIMEOUT_MS = 5_000
const WATCHDOG_INTERVAL_MS = 1_000

// The live socket, for components that need to send a command to the bridge
// (or hub) without owning a connection of their own.
let liveSocket: WebSocket | null = null

/** Send a command on the dashboard's main connection. False if it is down. */
export function sendBridgeCommand(command: object): boolean {
  if (!liveSocket || liveSocket.readyState !== WebSocket.OPEN) return false
  liveSocket.send(JSON.stringify(command))
  return true
}

export function useWebSocket() {
  const wsRef = useRef<WebSocket | null>(null)
  const reconnectAttemptRef = useRef(0)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastMessageAtRef = useRef(0)
  const connectStartedAtRef = useRef(0)
  // Abandons the current socket and schedules a reconnect (set by connect()).
  const retryRef = useRef<() => void>(() => {})
  const mountedRef = useRef(true)

  const setConnection = useTelemetryStore((s) => s.setConnection)
  const applyMessage = useTelemetryStore((s) => s.applyMessage)

  // Subscribe so a change of host, port or token triggers a reconnect.
  const wsHost = useSettingsStore((s) => s.wsHost)
  const wsPort = useSettingsStore((s) => s.wsPort)
  const bridgeToken = useSettingsStore((s) => s.bridgeToken)

  const connect = useCallback(() => {
    if (!mountedRef.current) return
    if (timerRef.current) { clearTimeout(timerRef.current); timerRef.current = null }

    const ws = new WebSocket(bridgeWsUrl())
    wsRef.current = ws
    connectStartedAtRef.current = Date.now()
    ws.binaryType = 'arraybuffer'

    // Leave this socket behind and try again after a backoff. Handlers are
    // detached first: closing a dead socket can take the browser a long time
    // to report, and we do not wait for it.
    const retry = () => {
      if (wsRef.current !== ws) return // already replaced
      ws.onopen = ws.onmessage = ws.onerror = ws.onclose = null
      try { ws.close() } catch { /* already closed */ }
      wsRef.current = null
      if (liveSocket === ws) liveSocket = null
      if (!mountedRef.current) return

      reconnectAttemptRef.current += 1
      const attempt = reconnectAttemptRef.current
      const delay = Math.min(BASE_DELAY_MS * Math.pow(2, attempt - 1), MAX_DELAY_MS)
        + Math.random() * JITTER_MS

      setConnection(attempt === 1 ? 'disconnected' : 'reconnecting')

      // Find out whether we are being refused rather than cut off. Retries
      // continue either way: a token fixed in Settings reconnects at once,
      // and a bridge restarted with another token simply stays "unauthorized".
      checkBridgeAuth().then((auth) => {
        if (auth === 'unauthorized' && mountedRef.current && wsRef.current?.readyState !== WebSocket.OPEN) {
          setConnection('unauthorized')
        }
      })

      timerRef.current = setTimeout(connect, delay)
    }

    ws.onopen = () => {
      if (!mountedRef.current) { ws.close(); return }
      reconnectAttemptRef.current = 0
      lastMessageAtRef.current = Date.now()
      liveSocket = ws
      setConnection('connected')
    }

    ws.onmessage = (event) => {
      if (!mountedRef.current) return
      lastMessageAtRef.current = Date.now()
      try {
        let msg: ServerMessage
        if (event.data instanceof ArrayBuffer) {
          // Binary frame → MessagePack
          msg = decode(new Uint8Array(event.data)) as ServerMessage
        } else {
          // Text frame → JSON fallback (debug mode)
          msg = JSON.parse(event.data as string) as ServerMessage
        }
        applyMessage(msg)
      } catch {
        // Malformed message — ignore
      }
    }

    ws.onerror = () => {
      // onclose fires right after; the retry is handled there.
    }

    ws.onclose = retry
    retryRef.current = retry
  }, [setConnection, applyMessage, wsHost, wsPort, bridgeToken])

  useEffect(() => {
    mountedRef.current = true
    reconnectAttemptRef.current = 0
    connect()

    // Dead in practice, whatever readyState claims: open but silent, or
    // still connecting after too long.
    const isStale = () => {
      const ws = wsRef.current
      if (!ws) return false
      if (ws.readyState === WebSocket.OPEN) {
        return Date.now() - lastMessageAtRef.current > STALE_AFTER_MS
      }
      if (ws.readyState === WebSocket.CONNECTING) {
        return Date.now() - connectStartedAtRef.current > CONNECT_TIMEOUT_MS
      }
      return false
    }
    const dropStale = () => retryRef.current()

    // Watchdog: a socket that has gone quiet, or never finished connecting,
    // is a dead link.
    const watchdog = setInterval(() => { if (isStale()) dropStale() }, WATCHDOG_INTERVAL_MS)

    // A tablet waking up, or the network coming back: do not wait out the
    // backoff, reconnect now. Timers are throttled or frozen while a page is
    // hidden, so the watchdog alone would react late.
    const wake = () => {
      if (document.visibilityState === 'hidden') return
      const ws = wsRef.current
      if (ws && !isStale()) return // healthy, or a fresh attempt in flight
      reconnectAttemptRef.current = 0
      if (ws) dropStale()
      else connect()
    }
    document.addEventListener('visibilitychange', wake)
    window.addEventListener('online', wake)
    window.addEventListener('pageshow', wake)

    return () => {
      mountedRef.current = false
      clearInterval(watchdog)
      document.removeEventListener('visibilitychange', wake)
      window.removeEventListener('online', wake)
      window.removeEventListener('pageshow', wake)
      if (timerRef.current) clearTimeout(timerRef.current)
      if (wsRef.current) {
        wsRef.current.onclose = null  // prevent reconnect on unmount
        wsRef.current.close()
        wsRef.current = null
      }
    }
  }, [connect])
}
