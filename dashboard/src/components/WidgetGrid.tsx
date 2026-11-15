import { useMemo, useState } from 'react'
import { ResponsiveGridLayout, useContainerWidth } from 'react-grid-layout'
import 'react-grid-layout/css/styles.css'
import 'react-resizable/css/styles.css'
import type { ResponsiveLayouts, Layout } from 'react-grid-layout'
import { deriveLayout } from '../utils/deriveLayout'
import { useLayoutStore } from '../stores/layoutStore'
import { WIDGET_REGISTRY } from './widgetRegistry'
import WidgetWrapper from './WidgetWrapper'

const BREAKPOINTS = { lg: 1200, md: 996, sm: 768 }
const COLS = { lg: 12, md: 10, sm: 6 }

export default function WidgetGrid() {
  const { layouts, widgets, scales, locked, setLayouts, removeWidget, setScale } = useLayoutStore()
  const { width, containerRef } = useContainerWidth({ initialWidth: 1280 })
  const [bp, setBp] = useState<string>('lg')

  // Narrow layouts are always derived from the wide one, so the two never
  // drift apart. Editing therefore happens at full width only.
  const effective = useMemo<ResponsiveLayouts>(() => {
    const lg = layouts.lg ?? []
    return { lg, md: deriveLayout(lg, COLS.md), sm: deriveLayout(lg, COLS.sm) }
  }, [layouts.lg])
  const editable = !locked && bp === 'lg'

  const handleLayoutChange = (_layout: Layout, allLayouts: ResponsiveLayouts) => {
    // Only the wide layout is the user's; the others are derived above.
    if (bp === 'lg' && allLayouts.lg) setLayouts({ lg: allLayouts.lg })
  }

  return (
    <div ref={containerRef} style={{ flex: 1, overflow: 'auto', minHeight: 0, WebkitOverflowScrolling: 'touch' }}>
      <ResponsiveGridLayout
        width={width}
        layouts={effective}
        breakpoints={BREAKPOINTS}
        cols={COLS}
        rowHeight={60}
        margin={[6, 6]}
        containerPadding={[6, 6]}
        dragConfig={{ enabled: editable, handle: '.widget-drag-handle', threshold: 3, bounded: false }}
        resizeConfig={{ enabled: editable, handles: ['se'] }}
        onLayoutChange={handleLayoutChange}
        onBreakpointChange={(newBp) => setBp(newBp)}
      >
        {widgets.map(({ id, widgetType }) => {
          const meta = WIDGET_REGISTRY[widgetType]
          if (!meta) return null
          const WidgetComponent = meta.component
          return (
            <div key={id}>
              <WidgetWrapper
                title={meta.name}
                locked={!editable}
                onRemove={() => removeWidget(id)}
                scale={scales[id] ?? 1}
                onScaleChange={(s) => setScale(id, s)}
              >
                <WidgetComponent />
              </WidgetWrapper>
            </div>
          )
        })}
      </ResponsiveGridLayout>
    </div>
  )
}
