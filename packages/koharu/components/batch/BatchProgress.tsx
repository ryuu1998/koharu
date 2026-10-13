'use client'

import { ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useBatchStore } from '@/lib/batch'
import { Progress } from '@koharu/ui/components/progress'

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
  const report = useBatchStore((s) => s.report)
  const [collapsed, setCollapsed] = useState<string | null>(null)
  if (!running)
    return (
      report && (
        <p className='mt-3 shrink-0 text-[11px]' role='status'>
          {report.stopped && <span className='mr-2 font-medium'>{t('batch.stopped')}</span>}
          {t('batch.summary', {
            completed: report.completed,
            skipped: report.skipped,
            failed: report.failures.length,
          })}
        </p>
      )
    )
  const expanded = path !== collapsed || !path
  const percent =
    progress && progress.total_steps ? (progress.completed_steps / progress.total_steps) * 100 : 0
  return (
    <section
      className='mt-3 shrink-0 overflow-hidden rounded-xl border border-border/80 bg-[var(--surface-panel)]'
      aria-label={t('batch.progress')}
    >
      <div className='flex items-center gap-3 px-4 py-3'>
        <button
          type='button'
          className='flex min-w-0 flex-1 items-center gap-2 text-left focus-visible:rounded focus-visible:outline-2 focus-visible:outline-ring'
          disabled={!path}
          aria-expanded={expanded}
          aria-controls='batch-active-details'
          aria-label={t(expanded ? 'batch.hideDetails' : 'batch.showDetails')}
          onClick={() => setCollapsed(expanded ? path : null)}
        >
          {expanded ? (
            <ChevronDown className='size-3.5 shrink-0' />
          ) : (
              <ChevronRight className='size-3.5 shrink-0' />
          )}
          <span className='truncate text-xs font-medium'>{active ?? t('batch.starting')}</span>
          <span className='ml-auto shrink-0 text-[11px] text-muted-foreground'>
            {t(
              stopping
                ? 'batch.stopping'
                : exporting
                  ? 'batch.exporting'
                  : progress
                    ? `phase.${progress.stage}`
                    : 'batch.starting',
            )}
          </span>
        </button>
        <span className='shrink-0 text-[10px] text-muted-foreground tabular-nums'>
          {t('batch.chapterCount', { count: done, total })}
        </span>
      </div>
      <Progress
        value={percent}
        aria-label={t('batch.chapterProgress')}
        className='h-0.5 rounded-none'
      />
      <div
        id='batch-active-details'
        hidden={!expanded}
        className='flex items-center gap-4 px-4 py-3'
      >
        {chapter?.thumbnail && (
          <img
            src={chapter.thumbnail}
            alt=''
            className='h-20 w-14 shrink-0 rounded-md object-cover'
          />
        )}
        <div className='grid min-w-0 flex-1 grid-cols-2 gap-x-5 gap-y-2'>
          {(['detection', 'ocr', 'translation', 'inpainting'] as const).map((stage) => {
            const current = stages[stage]
            const completed = current?.completed_pages ?? 0
            const pages = current?.total_pages ?? chapter?.pages ?? 0
            return (
              <div key={stage}>
                <div className='mb-1 flex items-center justify-between gap-2 text-[10px]'>
                  <span
                    className={
                      completed > 0 && completed === pages
                        ? 'text-green-700 dark:text-green-300'
                        : 'text-muted-foreground'
                    }
                  >
                    {t(`phase.${stage}`)}
                  </span>
                  <span className='text-muted-foreground tabular-nums'>
                    {completed} / {pages}
                  </span>
                </div>
                <Progress
                  value={pages ? (completed / pages) * 100 : 0}
                  aria-label={t(`phase.${stage}`)}
                  className='h-1'
                />
              </div>
            )
          })}
        </div>
      </div>
      <Progress
        value={total ? (done / total) * 100 : 0}
        aria-label={t('batch.progress')}
        className='h-0.5 rounded-none'
      />
    </section>
  )
}
