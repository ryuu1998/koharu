'use client'

import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { LoaderCircle, UploadCloud } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

export function PageDropZone({
  importing,
  onDrop,
  onBrowse,
}: {
  importing: boolean
  onDrop: (paths: string[]) => void
  onBrowse: () => void
}) {
  const { t } = useTranslation()
  const zone = useRef<HTMLButtonElement>(null)
  const latest = useRef({ importing, onDrop })
  latest.current = { importing, onDrop }
  const [dragging, setDragging] = useState(false)

  useEffect(() => {
    if (!isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (disposed) return
        if (payload.type === 'leave') {
          setDragging(false)
          return
        }
        const bounds = zone.current?.getBoundingClientRect()
        const scale = window.devicePixelRatio || 1
        const x = payload.position.x / scale
        const y = payload.position.y / scale
        const inside =
          !!bounds && x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom
        if (payload.type === 'drop') {
          setDragging(false)
          if (inside && !latest.current.importing && payload.paths.length)
            latest.current.onDrop(payload.paths)
        } else {
          setDragging(inside && !latest.current.importing)
        }
      })
      .then((stop) => {
        if (disposed) stop()
        else unlisten = stop
      })
      .catch(() => undefined)
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  return (
    <button
      ref={zone}
      type='button'
      disabled={importing}
      aria-busy={importing}
      onClick={onBrowse}
      aria-label={t('navigator.dropTitle')}
      data-dragging={dragging}
      className='mt-4 flex w-full max-w-48 flex-col items-center gap-1.5 rounded-xl border border-dashed border-border bg-background/40 px-3 py-4 text-center text-muted-foreground hover:border-primary/50 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50 data-[dragging=true]:border-primary data-[dragging=true]:bg-primary/10'
    >
      {importing ? (
        <LoaderCircle className='size-5 animate-spin' aria-hidden='true' />
      ) : (
        <UploadCloud className='size-5' aria-hidden='true' />
      )}
      <span className='text-[10px] font-medium'>
        {t(dragging ? 'navigator.dropActive' : 'navigator.dropTitle')}
      </span>
      <span className='text-[9px] leading-4'>{t('navigator.dropDescription')}</span>
    </button>
  )
}
