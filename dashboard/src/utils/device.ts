// Tablet helpers: keep the screen on, go full screen, tell touch devices apart.
//
// The dashboard is served over plain HTTP on the LAN or Tailscale. That rules
// out the Screen Wake Lock API, which browsers only offer on HTTPS (and on
// localhost). NoSleep.js covers the gap: it uses the Wake Lock API where it
// exists and otherwise plays a tiny muted looping video, which keeps Android
// and iPad screens on. Both must be started from a tap.

import NoSleep from 'nosleep.js'

let noSleep: NoSleep | null = null

export function isTouchDevice(): boolean {
  return typeof window !== 'undefined' && window.matchMedia?.('(pointer: coarse)').matches === true
}

export function keepAwakeActive(): boolean {
  return noSleep?.isEnabled ?? false
}

/** Must be called from a user gesture (a tap or click). */
export async function setKeepAwake(on: boolean): Promise<boolean> {
  if (!noSleep) noSleep = new NoSleep()
  try {
    if (on) await noSleep.enable()
    else noSleep.disable()
  } catch {
    return false
  }
  return noSleep.isEnabled
}

type FsDoc = Document & { webkitFullscreenElement?: Element; webkitExitFullscreen?: () => Promise<void> }
type FsEl = HTMLElement & { webkitRequestFullscreen?: () => Promise<void> }

export function isFullscreen(): boolean {
  const d = document as FsDoc
  return !!(d.fullscreenElement ?? d.webkitFullscreenElement)
}

/**
 * Toggle full screen, with the WebKit-prefixed calls iPad Safari still needs.
 * iPhone Safari has no element full screen at all: there, "Add to Home
 * Screen" is the way to lose the browser bars.
 */
export function toggleFullscreen(): void {
  const d = document as FsDoc
  const el = document.documentElement as FsEl
  if (isFullscreen()) {
    ;(d.exitFullscreen?.bind(d) ?? d.webkitExitFullscreen?.bind(d))?.()?.catch?.(() => {})
  } else {
    ;(el.requestFullscreen?.bind(el) ?? el.webkitRequestFullscreen?.bind(el))?.()?.catch?.(() => {})
  }
}

/** Running as an installed app (home screen), not in a browser tab. */
export function isStandalone(): boolean {
  const nav = navigator as Navigator & { standalone?: boolean }
  return window.matchMedia?.('(display-mode: standalone)').matches || window.matchMedia?.('(display-mode: fullscreen)').matches || nav.standalone === true
}
