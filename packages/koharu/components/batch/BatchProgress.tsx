'use client'

import { Check, ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useBatchStore } from '@/lib/batch'
import { Button } from '@koharu/ui/components/button'
import { Progress } from '@koharu/ui/components/progress'
import { cn } from '@koharu/ui/lib/utils'

export function BatchProgress() {
  const { t } = useTranslation()
  const running = useBatchStore((s) => s.running)
  const stopping = useBatchStore((s) => s.stopping)
  const progress = useBatchStore((s) => s.progress)
  const stages = useBatchStore((s) => s.stages)
  const active = useBatchStore((s) => s.activeName)
  const path = useBatchStore((s) => s.activePath)
  const chapter = useBatchStore((s) => s.source?.chapters.find((c) => c.path === s.activePath))
  const exporting = useBatchStore((s) => !!s.activePath && s.statuses[s.activePath] === 'exporting')
  const done = useBatchStore((s) => s.completedChapters)
  const total = useBatchStore((s) => s.totalChapters)
  const [collapsed, setCollapsed] = useState(false)
  if (!running) return null
  const expanded = !collapsed
  const percent = progress?.total_steps
    ? (progress.completed_steps / progress.total_steps) * 100
    : 0
  return (
    <aside
      aria-label={t('batch.progress')}
      className={cn(
        'order-first flex shrink-0 flex-col border-b border-border/70 bg-[var(--surface-panel)] min-[800px]:order-last min-[800px]:min-h-0 min-[800px]:border-s min-[800px]:border-b-0',
        expanded ? 'min-[800px]:w-[248px]' : 'min-[800px]:w-9',
      )}
    >
      <div
        className={cn(
          'flex h-11 shrink-0 items-center gap-2 border-b border-border/60 px-4',
          !expanded && 'min-[800px]:justify-center min-[800px]:px-1',
        )}
      >
        <h2
          className={cn(
            'min-w-0 flex-1 truncate text-[11px] font-medium',
            !expanded && 'min-[800px]:hidden',
          )}
        >
          {t('batch.progress')}
        </h2>
        <Button
          variant='ghost'
          size='icon-xs'
          disabled={!path}
          aria-expanded={expanded}
          aria-controls='batch-active-details'
          aria-label={t(expanded ? 'batch.hideDetails' : 'batch.showDetails')}
          title={t(expanded ? 'batch.hideDetails' : 'batch.showDetails')}
          className='text-muted-foreground aria-expanded:bg-transparent'
          onClick={() => setCollapsed(expanded)}
        >
          <ChevronDown
            className={cn(
              'size-3.5',
              expanded ? 'rotate-180 min-[800px]:-rotate-90' : 'min-[800px]:rotate-90',
            )}
            strokeWidth={1.75}
          />
        </Button>
      </div>
      {!expanded && (
        <p className='hidden py-3 text-center text-[9px] text-muted-foreground tabular-nums min-[800px]:block'>
          {Math.floor(percent)}%
        </p>
      )}
      <div
        id='batch-active-details'
        hidden={!expanded}
        className='min-h-0 flex-1 overflow-y-auto p-4'
      >
        <div className='mb-4'>
          <p className='mb-2 text-[10px] text-muted-foreground tabular-nums'>
            {t('batch.chapterCount', { count: done, total })}
          </p>
          <Progress
            value={total ? (done / total) * 100 : 0}
            aria-label={t('batch.progress')}
            className='[&_[data-slot=progress-track]]:h-0.5'
          />
        </div>
        <div className='grid grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)] gap-5 min-[800px]:block'>
          <div>
            <div className='flex items-start gap-3'>
              {chapter?.thumbnail && (
                <img
                  src={chapter.thumbnail}
                  alt=''
                  className='h-14 w-10 shrink-0 rounded object-contain'
                />
              )}
              <div className='min-w-0 flex-1'>
                <p
                  className='line-clamp-3 text-xs leading-[18px] font-medium break-words'
                  title={active ?? undefined}
                >
                  {active?.replace(/\.cbz$/i, '') ?? t('batch.starting')}
                </p>
                <p role='status' className='mt-1 text-[10px] leading-4 text-muted-foreground'>
                  {t(stopping ? 'batch.stopping' : exporting ? 'batch.exporting' : 'batch.running')}
                </p>
              </div>
            </div>
            <div className='mt-4'>
              <div className='mb-2 flex items-center justify-between gap-2 text-[10px] text-muted-foreground'>
                <span>{t('batch.chapterProgress')}</span>
                <span className='tabular-nums'>{Math.floor(percent)}%</span>
              </div>
              <Progress
                value={percent}
                aria-label={t('batch.chapterProgress')}
                className='[&_[data-slot=progress-track]]:h-0.5'
              />
            </div>
          </div>
          <div className='grid grid-cols-2 gap-x-4 gap-y-4 min-[800px]:mt-6 min-[800px]:grid-cols-1 min-[800px]:gap-y-5'>
            {(['detection', 'ocr', 'translation', 'inpainting'] as const).map((stage) => {
              const current = stages[stage]
              const completed = current?.completed_pages ?? 0
              const pages = current?.total_pages ?? chapter?.pages ?? 0
              const finished = pages > 0 && completed === pages
              return (
                <div key={stage}>
                  <div className='mb-2 flex items-center justify-between gap-2 text-[11px]'>
                    <span className='flex items-center gap-1.5'>
                      {t('phase.' + stage)}
                      {finished && (
                        <Check
                          className='size-3 text-green-700 dark:text-green-300'
                          strokeWidth={1.75}
                          aria-hidden='true'
                        />
                      )}
                    </span>
                    <span className='text-[10px] text-muted-foreground tabular-nums'>
                      {completed} / {pages}
                    </span>
                  </div>
                  <Progress
                    value={pages ? (completed / pages) * 100 : 0}
                    aria-label={t('phase.' + stage)}
                    className={cn(
                      '[&_[data-slot=progress-track]]:h-0.5',
                      finished && '[&_[data-slot=progress-indicator]]:bg-foreground/25',
                    )}
                  />
                </div>
              )
            })}
          </div>
        </div>
      </div>
    </aside>
  )
}
