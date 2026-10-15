'use client'

import {
  Trash2,
  FileArchive,
  GripVertical,
  PanelRightClose,
  PanelRightOpen,
  TriangleAlert,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BatchStageProgress } from '@/components/batch/BatchStageProgress'
import { removeBatch, reorderBatch, retryBatch, useBatchStore } from '@/lib/batch'
import { Button } from '@koharu/ui/components/button'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@koharu/ui/components/dialog'
import { Progress } from '@koharu/ui/components/progress'
import { cn } from '@koharu/ui/lib/utils'

export function BatchQueue() {
  const { t } = useTranslation()
  const source = useBatchStore((s) => s.source)
  const selected = useBatchStore((s) => s.selected)
  const queue = useBatchStore((s) => s.queue)
  const exported = useBatchStore((s) => s.exported)
  const statuses = useBatchStore((s) => s.statuses)
  const errors = useBatchStore((s) => s.errors)
  const running = useBatchStore((s) => s.running)
  const stopping = useBatchStore((s) => s.stopping)
  const pauseState = useBatchStore((s) => s.pauseState)
  const activePath = useBatchStore((s) => s.activePath)
  const progress = useBatchStore((s) => s.progress)
  const rendering = useBatchStore((s) => s.rendering)
  const [collapsed, setCollapsed] = useState(false)
  const [following, setFollowing] = useState(true)
  const [dragged, setDragged] = useState<string | null>(null)
  const [dragY, setDragY] = useState(0)
  const [dropBefore, setDropBefore] = useState<string | null>(null)
  const [errorPath, setErrorPath] = useState<string | null>(null)
  const [retrying, setRetrying] = useState<string | null>(null)
  const drag = useRef<{
    path: string
    start: number
    top: number
    before: string | null
    moved: boolean
  } | null>(null)
  const viewport = useRef<HTMLDivElement>(null)
  const [viewportHeight, setViewportHeight] = useState(0)
  const chapters = useMemo(() => new Map(source?.chapters.map((c) => [c.path, c])), [source])
  const paths = running || !selected.length ? queue : selected
  const hasPaths = paths.length > 0
  const activeIndex = paths.findIndex(
    (path) => path === activePath && ['running', 'exporting'].includes(statuses[path]),
  )
  const next = activeIndex >= 0 ? activeIndex : paths.findIndex((path) => !statuses[path])
  const focusIndex = !running && selected.length ? 0 : next < 0 ? paths.length : next
  const movable = (path: string, index: number) =>
    !stopping && (!running || (!statuses[path] && index > focusIndex))
  const processed = paths.filter((path) =>
    ['translated', 'skipped', 'failed', 'stopped'].includes(statuses[path]),
  ).length
  const percent = progress?.total_steps
    ? Math.max(
        0,
        Math.min(
          99,
          Math.floor(
            ((progress.completed_steps + (rendering?.completed_pages ?? 0)) /
              (progress.total_steps + progress.total_pages)) *
              100,
          ),
        ),
      )
    : 0

  useEffect(() => {
    setFollowing(true)
  }, [activePath])

  useEffect(() => {
    const panel = viewport.current
    if (!panel) return
    const observer = new ResizeObserver(() => setViewportHeight(panel.clientHeight))
    observer.observe(panel)
    return () => observer.disconnect()
  }, [hasPaths])

  useEffect(() => {
    if (following && !collapsed && viewport.current) {
      viewport.current.scrollTo({
        top: Math.min(focusIndex, Math.max(0, paths.length - 1)) * 48,
        behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth',
      })
    }
  }, [focusIndex, following, collapsed, viewportHeight, paths.length])

  if (!paths.length) return null

  return (
    <aside
      aria-label={t('batch.queue')}
      className={cn(
        'flex min-h-0 shrink-0 flex-col border-t border-border/70 bg-[var(--surface-panel)] min-[800px]:border-s min-[800px]:border-t-0',
        collapsed
          ? 'h-10 min-[800px]:h-auto min-[800px]:w-9'
          : 'h-[420px] max-h-[70vh] min-[800px]:h-auto min-[800px]:max-h-none min-[800px]:w-[280px]',
      )}
    >
      <div
        className={cn(
          'flex h-11 shrink-0 items-center gap-2 px-4',
          collapsed && 'min-[800px]:justify-center min-[800px]:px-1',
        )}
      >
        <h2 className={cn('min-w-0 flex-1 text-xs font-medium', collapsed && 'min-[800px]:hidden')}>
          {t('batch.queue')}
        </h2>
        {!collapsed && (
          <span className='text-[10px] text-muted-foreground tabular-nums'>
            {processed} / {paths.length}
          </span>
        )}
        <Button
          variant='ghost'
          size='icon-xs'
          aria-expanded={!collapsed}
          aria-controls='batch-chapter-queue'
          aria-label={t(collapsed ? 'batch.showQueue' : 'batch.hideQueue')}
          title={t(collapsed ? 'batch.showQueue' : 'batch.hideQueue')}
          className='text-muted-foreground'
          onClick={() => setCollapsed(!collapsed)}
        >
          {collapsed ? (
            <PanelRightOpen className='size-3.5' />
          ) : (
            <PanelRightClose className='size-3.5' />
          )}
        </Button>
      </div>
      {!collapsed && !following && !dragged && running && (
        <Button
          variant='ghost'
          size='xs'
          className='mx-4 mb-2 self-start text-[11px]'
          onClick={() => setFollowing(true)}
        >
          {t('batch.followActive')}
        </Button>
      )}
      <div
        id='batch-chapter-queue'
        ref={viewport}
        hidden={collapsed}
        className='min-h-0 flex-1 overflow-y-auto px-4 pb-4'
        onWheel={() => setFollowing(false)}
        onPointerDown={(event) => {
          if (event.target === event.currentTarget) setFollowing(false)
        }}
        onTouchMove={() => setFollowing(false)}
        onKeyDown={(event) => {
          if (['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key))
            setFollowing(false)
        }}
      >
        <ol
          className='relative'
          aria-label={t('batch.queueChapters')}
          style={{
            height:
              focusIndex < paths.length
                ? Math.max(paths.length * 48 + 96, focusIndex * 48 + viewportHeight - 16)
                : paths.length * 48 + 8,
          }}
        >
          {paths.map((path, index) => {
            const chapter = chapters.get(path)
            if (!chapter) return null
            const focused = index === focusIndex
            const active = path === activePath && ['running', 'exporting'].includes(statuses[path])
            const done = exported.includes(path)
            const failed = statuses[path] === 'failed'
            const caption = done
              ? 'batch.translated'
              : active && pauseState !== 'playing' && !stopping
                ? 'batch.' + pauseState
                : stopping && active
                  ? 'batch.stopping'
                  : statuses[path]
                    ? `batch.${statuses[path]}`
                    : focused && !running
                      ? 'batch.upNext'
                      : 'batch.waiting'
            const canMove = movable(path, index)
            const target = dragged && dropBefore === path
            const top = 8 + index * 48 + (index > focusIndex ? 88 : 0)
            return (
              <li
                key={path}
                aria-label={chapter.name}
                aria-current={active ? 'step' : undefined}
                data-queue-path={path}
                data-movable={canMove}
                data-dragging={dragged === path}
                draggable={false}
                onPointerDown={(event) => {
                  if (
                    !canMove ||
                    event.button !== 0 ||
                    (event.target as HTMLElement).closest('button')
                  )
                    return
                  event.currentTarget.setPointerCapture(event.pointerId)
                  drag.current = { path, start: event.clientY, top, before: path, moved: false }
                }}
                onPointerMove={(event) => {
                  const current = drag.current
                  if (!current || current.path !== path || !canMove) return
                  if (!current.moved && Math.abs(event.clientY - current.start) < 4) return
                  current.moved = true
                  setDragged(path)
                  setFollowing(false)
                  const panel = viewport.current
                  if (panel) {
                    const bounds = panel.getBoundingClientRect()
                    if (event.clientY < bounds.top + 32) panel.scrollBy({ top: -16 })
                    else if (event.clientY > bounds.bottom - 32) panel.scrollBy({ top: 16 })
                    const before =
                      paths.find((entry, slot) => {
                        if (entry === path || !movable(entry, slot)) return false
                        const slotTop = 8 + slot * 48 + (slot > focusIndex ? 88 : 0)
                        const midpoint = slotTop + (slot === focusIndex ? 68 : 24)
                        return event.clientY - bounds.top + panel.scrollTop < midpoint
                      }) ?? null
                    current.before = before
                    setDropBefore(before)
                  }
                  setDragY(current.top + event.clientY - current.start)
                }}
                onPointerUp={(event) => {
                  const current = drag.current
                  drag.current = null
                  setDragged(null)
                  if (event.currentTarget.hasPointerCapture(event.pointerId))
                    event.currentTarget.releasePointerCapture(event.pointerId)
                  if (current?.moved && canMove)
                    void reorderBatch(current.path, current.before).catch(() => undefined)
                }}
                onPointerCancel={() => {
                  drag.current = null
                  setDragged(null)
                }}
                onLostPointerCapture={() => {
                  drag.current = null
                  setDragged(null)
                }}
                className={cn(
                  'batch-queue-item absolute inset-x-0',
                  canMove && 'group cursor-grab touch-none select-none active:cursor-grabbing',
                  dragged === path && 'z-10 rounded bg-[var(--surface-panel)] shadow-md',
                )}
                style={{
                  transform: `translateY(${dragged === path ? dragY : top}px)`,
                  transition: dragged === path ? 'none' : undefined,
                }}
              >
                {target && (
                  <span
                    aria-hidden='true'
                    className='absolute inset-x-0 -top-1 h-0.5 rounded bg-primary'
                  />
                )}
                <div className='flex items-start gap-3'>
                  <span
                    className={cn(
                      'batch-queue-cover relative grid shrink-0 place-items-center overflow-hidden rounded bg-[var(--surface-well)]',
                      done && 'ring-1 ring-green-600/35',
                    )}
                    style={{ width: focused ? 52.8 : 21.6, height: focused ? 70.8 : 28.8 }}
                  >
                    {chapter.thumbnail ? (
                      <img
                        src={chapter.thumbnail}
                        alt=''
                        draggable={false}
                        className='absolute inset-0 size-full object-contain'
                      />
                    ) : (
                      <FileArchive
                        className={cn('text-muted-foreground/40', focused ? 'size-5' : 'size-3')}
                      />
                    )}
                  </span>
                  <div className='min-w-0 flex-1'>
                    <p
                      title={chapter.name}
                      className={cn(
                        'font-medium',
                        focused
                          ? 'line-clamp-2 text-xs leading-[18px] break-words'
                          : 'truncate text-[11px] leading-4',
                      )}
                    >
                      {chapter.name.replace(/\.cbz$/i, '')}
                    </p>
                    <p
                      className={cn(
                        'mt-1 flex items-center gap-1 text-[10px] leading-4',
                        done
                          ? 'text-green-700 dark:text-green-300'
                          : failed
                            ? 'text-destructive'
                            : active
                              ? 'text-primary'
                              : 'text-muted-foreground',
                      )}
                    >
                      {done && (
                        <span
                          className='size-1.5 shrink-0 rounded-full bg-green-600 dark:bg-green-400'
                          aria-hidden='true'
                        />
                      )}
                      {failed && <TriangleAlert className='size-3 shrink-0' aria-hidden='true' />}
                      {failed && errors[path] ? (
                        <button
                          className='underline decoration-destructive/40 underline-offset-2'
                          onClick={() => setErrorPath(path)}
                          aria-label={t('batch.showError', { chapter: chapter.name })}
                        >
                          {t(caption)}
                        </button>
                      ) : (
                        t(caption)
                      )}
                    </p>
                  </div>
                  {failed && (
                    <Button
                      variant='ghost'
                      size='xs'
                      className='h-6 px-1.5 text-[10px]'
                      disabled={stopping || retrying !== null}
                      aria-label={t('batch.retryChapter', { chapter: chapter.name })}
                      onClick={() => {
                        setRetrying(path)
                        void retryBatch(path)
                          .catch(() => undefined)
                          .finally(() => setRetrying(null))
                      }}
                    >
                      {t('batch.retry')}
                    </Button>
                  )}
                  {canMove && selected.includes(path) && (
                    <Button
                      variant='ghost'
                      size='icon-xs'
                      aria-label={t('batch.removeFromQueue', { chapter: chapter.name })}
                      title={t('batch.removeFromQueue', { chapter: chapter.name })}
                      className='shrink-0 text-muted-foreground hover:text-destructive'
                      onClick={() => void removeBatch(path).catch(() => undefined)}
                    >
                      <Trash2 className='size-3.5' strokeWidth={1.75} />
                    </Button>
                  )}
                  {canMove && (
                    <GripVertical
                      className='mt-1 size-3 shrink-0 text-muted-foreground/40 opacity-0 group-hover:opacity-100'
                      aria-label={t('batch.dragToReorder')}
                    />
                  )}
                </div>
                {active && (
                  <div className='absolute inset-x-0 top-[90px]'>
                    <Progress
                      value={percent}
                      aria-label={t('batch.chapterProgress')}
                      className='[&_[data-slot=progress-track]]:h-1'
                    />
                    <div className='mt-1.5 flex items-center justify-between gap-2 text-[10px] text-muted-foreground'>
                      <span>{t('batch.chapterProgress')}</span>
                      <span className='tabular-nums'>{percent}%</span>
                    </div>
                  </div>
                )}
              </li>
            )
          })}
          {dragged && dropBefore === null && (
            <span
              aria-hidden='true'
              className='absolute inset-x-0 bottom-0 h-0.5 rounded bg-primary'
            />
          )}
        </ol>
      </div>
      {!collapsed && <BatchStageProgress path={paths[focusIndex]} />}
      <Dialog
        open={errorPath !== null}
        onOpenChange={(open) => {
          if (!open) setErrorPath(null)
        }}
      >
        <DialogContent>
          <DialogTitle>{t('batch.chapterFailed')}</DialogTitle>
          <DialogDescription className='break-words'>
            {errorPath ? chapters.get(errorPath)?.name : ''}
          </DialogDescription>
          <p className='max-h-64 overflow-y-auto text-xs leading-relaxed break-words whitespace-pre-wrap'>
            {errorPath ? errors[errorPath] : ''}
          </p>
        </DialogContent>
      </Dialog>
    </aside>
  )
}
