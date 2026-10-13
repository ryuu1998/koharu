'use client'

import { useTranslation } from 'react-i18next'

import { useBatchStore } from '@/lib/batch'
import { Progress } from '@koharu/ui/components/progress'

export function BatchProgress() {
  const { t } = useTranslation()
  const running = useBatchStore((state) => state.running)
  const stopping = useBatchStore((state) => state.stopping)
  const progress = useBatchStore((state) => state.progress)
  const active = useBatchStore((state) => state.activeName)
  const exporting = useBatchStore(
    (state) => !!state.activePath && state.statuses[state.activePath] === 'exporting',
  )
  const done = useBatchStore((state) => state.completedChapters)
  const total = useBatchStore((state) => state.totalChapters)
  const report = useBatchStore((state) => state.report)
  if (!running && !report) return null

  return (
    <section
      className='mt-3 shrink-0 rounded-xl border border-border/80 bg-[var(--surface-panel)] px-4 py-3'
      aria-label={t('batch.progress')}
    >
      {running ? (
        <>
          <div
            className='mb-2 flex min-w-0 items-center justify-between gap-4 text-[11px]'
            role='status'
            aria-live='polite'
          >
            <span className='truncate font-medium'>{active ?? t('batch.starting')}</span>
            <span className='shrink-0 text-muted-foreground'>
              {stopping
                ? t('batch.stopping')
                : exporting
                  ? t('batch.exporting')
                  : progress
                    ? t(`phase.${progress.stage}`)
                    : t('batch.starting')}
            </span>
          </div>
          <div className='grid gap-3 sm:grid-cols-2'>
            <div>
              <div className='mb-1.5 flex justify-between text-[10px] text-muted-foreground'>
                <span>{t('batch.chapterProgress')}</span>
                <span className='tabular-nums'>
                  {progress
                    ? t('batch.pageProgress', {
                        count: progress.completed_pages,
                        total: progress.total_pages,
                      })
                    : t('common.loading')}
                </span>
              </div>
              <Progress
                value={
                  progress && progress.total_steps
                    ? (progress.completed_steps / progress.total_steps) * 100
                    : 0
                }
                aria-label={t('batch.chapterProgress')}
              />
            </div>
            <div>
              <div className='mb-1.5 flex justify-between text-[10px] text-muted-foreground'>
                <span>{t('batch.progress')}</span>
                <span className='tabular-nums'>
                  {t('batch.chapterCount', { count: done, total })}
                </span>
              </div>
              <Progress value={total ? (done / total) * 100 : 0} aria-label={t('batch.progress')} />
            </div>
          </div>
        </>
      ) : (
        report && (
          <p className='text-[11px]' role='status'>
            {report.stopped && <span className='mr-2 font-medium'>{t('batch.stopped')}</span>}
            {t('batch.summary', {
              completed: report.completed,
              skipped: report.skipped,
              failed: report.failures.length,
            })}
          </p>
        )
      )}
    </section>
  )
}
