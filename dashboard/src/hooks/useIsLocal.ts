import { useAccess } from './useAccess'

/**
 * Is this page open on the bridge's (or hub's) own PC? Some actions are only
 * accepted from there, and the bridge refuses them from anywhere else, so the
 * controls are shown only where they work. False until the bridge has answered.
 */
export function useIsLocal(): boolean {
  return useAccess().local
}
