'use client'

import { useTranslation } from 'react-i18next'

import { useBatchStore } from '@/lib/batch'
import { Progress } from '@koharu/ui/components/progress'

export function BatchStageProgress({ path }: { path: string | undefined }) {
  const { t } = useTranslation()
  const activePath = useBatchStore((s) => s.activePath)
  const running = useBatchStore((s) => s.running)
  const stages = useBatchStore((s) => s.stages)
  const activity = useBatchStore((s) => s.stageActivity)
  const paused = useBatchStore((s) => s.pauseState === 'paused')
  const rendering = useBatchStore((s) => s.rendering)
  const progress = useBatchStore((s) => s.progress)
  const source = useBatchStore((s) => s.source)
  if (!running || !path) return null
  const current = activePath === path
  const total =
    (current ? progress?.total_pages : undefined) ??
    source?.chapters.find((c) => c.path === path)?.pages ??
    0

  return (
    <div
      className='flex shrink-0 flex-col gap-3 border-t border-border/70 px-4 py-3'
      aria-label={t('batch.stageProgress')}
    >
      {(['detection', 'ocr', 'inpainting', 'translation', 'rendering'] as const).map((stage) => {
        const count = current
          ? Math.min(
              total,
              stage === 'rendering' ? (rendering?.completed_pages ?? 0) : (stages[stage] ?? 0),
            )
          : 0
        const name = t(stage === 'rendering' ? 'batch.rendering' : `phase.${stage}`)
        const active = current && !paused && stage !== 'rendering' ? (activity[stage] ?? 0) : 0
        return (
          <div key={stage} className='min-w-0'>
            <div className='mb-1.5 flex items-center justify-between gap-1 text-[10px]'>
              <span className='truncate text-muted-foreground'>{name}</span>
              <div className='flex shrink-0 items-center gap-2 tabular-nums'>
                {active > 0 && (
                  <span className='text-primary' role='status'>
                    {t('batch.activePages', { count: active })}
                  </span>
                )}
                <span>
                  {count}/{total}
                </span>
              </div>
            </div>
            <Progress
              value={total ? (count / total) * 100 : 0}
              aria-label={name}
              aria-valuetext={t('batch.pageProgress', { count, total })}
              className='[&_[data-slot=progress-track]]:h-1'
            />
          </div>
        )
      })}
    </div>
  )
}
