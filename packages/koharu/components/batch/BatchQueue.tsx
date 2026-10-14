'use client'

import { Check, FileArchive, PanelRightClose, PanelRightOpen, TriangleAlert } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { sortBatchChapters, useBatchStore } from '@/lib/batch'
import { Button } from '@koharu/ui/components/button'
import { Progress } from '@koharu/ui/components/progress'
import { cn } from '@koharu/ui/lib/utils'

export function BatchQueue() {
  const { t } = useTranslation()
  const source = useBatchStore((s) => s.source)
  const selected = useBatchStore((s) => s.selected)
  const queue = useBatchStore((s) => s.queue)
  const exported = useBatchStore((s) => s.exported)
  const statuses = useBatchStore((s) => s.statuses)
  const running = useBatchStore((s) => s.running)
  const stopping = useBatchStore((s) => s.stopping)
  const activePath = useBatchStore((s) => s.activePath)
  const progress = useBatchStore((s) => s.progress)
  const sortBy = useBatchStore((s) => s.sortBy)
  const descending = useBatchStore((s) => s.sortDescending)
  const [collapsed, setCollapsed] = useState(false)
  const [following, setFollowing] = useState(true)
  const viewport = useRef<HTMLDivElement>(null)
  const chapters = useMemo(() => new Map(source?.chapters.map((c) => [c.path, c])), [source])
  const paths = queue.length
    ? queue
    : sortBatchChapters(source?.chapters ?? [], sortBy, descending)
        .filter((c) => !c.error && selected.includes(c.path))
        .map((c) => c.path)
  const next = paths.findIndex(
    (path) => ['running', 'exporting'].includes(statuses[path]) || !statuses[path],
  )
  const focusIndex = next < 0 ? paths.length : next
  const processed = paths.filter((path) =>
    ['translated', 'skipped', 'failed', 'stopped'].includes(statuses[path]),
  ).length
  const percent = progress?.total_steps
    ? Math.max(0, Math.min(99, Math.floor((progress.completed_steps / progress.total_steps) * 100)))
    : 0

  useEffect(() => {
    if (following && !collapsed && viewport.current) {
      viewport.current.scrollTo({
        top: Math.max(0, (focusIndex - 1) * 48),
        behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth',
      })
    }
  }, [focusIndex, following, collapsed])

  if (!paths.length) return null

  return (
    <aside
      aria-label={t('batch.queue')}
      className={cn(
        'flex min-h-0 shrink-0 flex-col border-t border-border/70 bg-[var(--surface-panel)] min-[800px]:border-s min-[800px]:border-t-0',
        collapsed
          ? 'h-10 min-[800px]:h-auto min-[800px]:w-9'
          : 'h-72 min-[800px]:h-auto min-[800px]:w-[280px]',
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
      {!collapsed && !following && (
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
        onPointerDown={() => setFollowing(false)}
        onTouchMove={() => setFollowing(false)}
        onKeyDown={(event) => {
          if (['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp', 'Home', 'End'].includes(event.key))
            setFollowing(false)
        }}
      >
        <ol
          className='relative'
          aria-label={t('batch.queueChapters')}
          style={{ height: paths.length * 48 + (focusIndex < paths.length ? 88 : 0) + 8 }}
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
              : stopping && active
                ? 'batch.stopping'
                : statuses[path]
                  ? `batch.${statuses[path]}`
                  : focused && !running
                    ? 'batch.upNext'
                    : 'batch.waiting'
            const top = 8 + index * 48 + (index > focusIndex ? 88 : 0)
            return (
              <li
                key={path}
                aria-label={chapter.name}
                aria-current={active ? 'step' : undefined}
                className='batch-queue-item absolute inset-x-0'
                style={{ transform: `translateY(${top}px)` }}
              >
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
                      {done && <Check className='size-3 shrink-0' aria-hidden='true' />}
                      {failed && <TriangleAlert className='size-3 shrink-0' aria-hidden='true' />}
                      {t(caption)}
                    </p>
                  </div>
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
        </ol>
      </div>
    </aside>
  )
}
