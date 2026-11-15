import { colors, fonts } from '../styles/theme'

/**
 * Shown instead of fuel and tyre figures for a car followed from the engineer
 * seat: LMU publishes placeholders for another PC's car (half a tank, new
 * tyres), and showing them as data would be worse than showing nothing.
 */
export default function SpectatedNotice({ what }: { what: string }) {
  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', justifyContent: 'center', alignItems: 'center', gap: 6, padding: 10, boxSizing: 'border-box', textAlign: 'center' }}>
      <span style={{ fontFamily: fonts.body, fontSize: 15, color: colors.textMuted, letterSpacing: 1, textTransform: 'uppercase' }}>{what}</span>
      <span style={{ fontFamily: fonts.body, fontSize: 13, color: colors.textMuted, lineHeight: 1.35 }}>
        Not available from the engineer seat: the game does not share this car's {what.toLowerCase()} with other PCs.
        It needs Pitwall running on the driver's PC.
      </span>
    </div>
  )
}
