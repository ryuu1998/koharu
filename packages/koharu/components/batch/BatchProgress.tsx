'use client'

import { Check, PanelRightClose, PanelRightOpen } from 'lucide-react'
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
        'order-first flex shrink-0 flex-col border-b border-border/70 bg-muted/40 min-[900px]:order-last min-[900px]:min-h-0 min-[900px]:border-s min-[900px]:border-b-0',
        expanded ? 'min-[900px]:w-[280px] lg:w-[304px] xl:w-[328px]' : 'min-[900px]:w-14',
      )}
    >
      <div className={cn('flex items-center gap-2 px-5 py-4', !expanded && 'min-[900px]:px-3')}>
        <div className={cn('min-w-0 flex-1', !expanded && 'min-[900px]:hidden')}>
          <h2 className='text-[13px] font-semibold'>{t('batch.progress')}</h2>
          <p className='mt-1 text-[11px] text-muted-foreground tabular-nums'>
            {t('batch.chapterCount', { count: done, total })}
          </p>
          <Progress
            value={total ? (done / total) * 100 : 0}
            aria-label={t('batch.progress')}
            className='mt-3 [&_[data-slot=progress-track]]:h-0.5'
          />
        </div>
        <Button
          variant='ghost'
          size='icon'
          disabled={!path}
          aria-expanded={expanded}
          aria-controls='batch-active-details'
          aria-label={t(expanded ? 'batch.hideDetails' : 'batch.showDetails')}
          title={t(expanded ? 'batch.hideDetails' : 'batch.showDetails')}
          className='text-muted-foreground aria-expanded:bg-transparent'
          onClick={() => setCollapsed(expanded)}
        >
          {expanded ? (
            <PanelRightClose className='size-[18px]' strokeWidth={1.5} />
          ) : (
            <PanelRightOpen className='size-[18px]' strokeWidth={1.5} />
          )}
        </Button>
      </div>
      {!expanded && (
        <p className='hidden px-3 py-2 text-center text-[11px] text-muted-foreground tabular-nums min-[900px]:block'>
          {Math.floor(percent)}%
        </p>
      )}
      <div
        id='batch-active-details'
        hidden={!expanded}
        className='min-h-0 overflow-y-auto px-5 pb-6 min-[900px]:px-6'
      >
        <div className='grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-x-6 min-[900px]:block'>
          <div>
            <div className='flex items-center gap-4 min-[900px]:mt-5 min-[900px]:flex-col min-[900px]:items-start min-[900px]:gap-5 min-[900px]:[@media(max-height:760px)]:mt-2 min-[900px]:[@media(max-height:760px)]:flex-row min-[900px]:[@media(max-height:760px)]:items-center min-[900px]:[@media(max-height:760px)]:gap-4'>
              {chapter?.thumbnail && (
                <img
                  src={chapter.thumbnail}
                  alt=''
                  className='hidden h-40 max-w-32 rounded-md object-contain object-left shadow-sm min-[900px]:block min-[900px]:[@media(max-height:760px)]:h-20 min-[900px]:[@media(max-height:760px)]:max-w-16'
                />
              )}
              <div className='min-w-0'>
                <p
                  className='line-clamp-3 text-[15px] leading-snug font-medium break-words'
                  title={active ?? undefined}
                >
                  {active?.replace(/\.cbz$/i, '') ?? t('batch.starting')}
                </p>
                <p role='status' className='mt-2 text-xs text-muted-foreground'>
                  {t(stopping ? 'batch.stopping' : exporting ? 'batch.exporting' : 'batch.running')}
                </p>
              </div>
            </div>
            <div className='mt-4 min-[900px]:mt-7 min-[900px]:[@media(max-height:760px)]:mt-4'>
              <div className='mb-2 flex items-center justify-between gap-3 text-[11px] text-muted-foreground'>
                <span>{t('batch.chapterProgress')}</span>
                <span className='tabular-nums'>{Math.floor(percent)}%</span>
              </div>
              <Progress
                value={percent}
                aria-label={t('batch.chapterProgress')}
                className='[&_[data-slot=progress-track]]:h-1.5'
              />
            </div>
          </div>
          <div className='grid grid-cols-2 gap-x-5 gap-y-5 min-[900px]:mt-8 min-[900px]:grid-cols-1 min-[900px]:gap-y-6 min-[900px]:[@media(max-height:760px)]:mt-6 min-[900px]:[@media(max-height:760px)]:gap-y-5'>
            {(['detection', 'ocr', 'translation', 'inpainting'] as const).map((stage) => {
              const current = stages[stage]
              const completed = current?.completed_pages ?? 0
              const pages = current?.total_pages ?? chapter?.pages ?? 0
              const finished = pages > 0 && completed === pages
              return (
                <div key={stage}>
                  <div className='mb-2 flex items-center justify-between gap-3 text-xs'>
                    <span className='flex items-center gap-2'>
                      {t('phase.' + stage)}
                      {finished && (
                        <Check
                          className='size-3.5 text-green-700 dark:text-green-300'
                          strokeWidth={2}
                          aria-hidden='true'
                        />
                      )}
                    </span>
                    <span className='text-[11px] text-muted-foreground tabular-nums'>
                      {completed} / {pages}
                    </span>
                  </div>
                  <Progress
                    value={pages ? (completed / pages) * 100 : 0}
                    aria-label={t('phase.' + stage)}
                    className={cn(
                      finished && '[&_[data-slot=progress-indicator]]:bg-foreground/30',
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
