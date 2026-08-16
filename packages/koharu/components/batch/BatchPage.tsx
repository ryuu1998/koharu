'use client'

import { Channel } from '@tauri-apps/api/core'
import {
  ArrowLeft,
  Check,
  FolderInput,
  FolderOpen,
  FolderOutput,
  Images,
  LoaderCircle,
  Play,
  Square,
  TriangleAlert,
} from 'lucide-react'
import Image from 'next/image'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { useKoharuStore, type BatchChapterStatus } from '@/lib/store'
import { commands, type BatchChapter, type BatchEvent } from '@koharu/bridge/protocol'
import { Badge } from '@koharu/ui/components/badge'
import { Button } from '@koharu/ui/components/button'
import { Progress, ProgressLabel } from '@koharu/ui/components/progress'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { cn } from '@koharu/ui/lib/utils'

export function BatchPage() {
  const { t } = useTranslation()
  const batch = useKoharuStore((state) => state.batch)
  const setBatchOpen = useKoharuStore((state) => state.setBatchOpen)
  const updateBatch = useKoharuStore((state) => state.updateBatch)
  const [loadingSource, setLoadingSource] = useState(false)
  const [loadingOutput, setLoadingOutput] = useState(false)

  const validChapters = batch.source?.chapters.filter((chapter) => !chapter.error) ?? []
  const selected = useMemo(() => new Set(batch.selected), [batch.selected])
  const selectedChapters = validChapters.filter((chapter) => selected.has(chapter.path))
  const current = currentProgress(batch.event)
  const currentPath = current ? batch.selected[current.index] : undefined
  const currentChapter = batch.source?.chapters.find((chapter) => chapter.path === currentPath)
  const currentPercent =
    current?.event === 'chapter_progress'
      ? percentage(current.completed_steps, current.total_steps)
      : current?.event === 'chapter_finished' || current?.event === 'chapter_skipped'
        ? 100
        : 0
  const settled = batch.selected.filter((path) => {
    const status = batch.statuses[path]
    return status === 'completed' || status === 'skipped' || status === 'failed'
  }).length
  const activeFraction =
    batch.running && current?.event === 'chapter_progress' ? currentPercent / 100 : 0
  const overallPercent =
    batch.event?.event === 'finished' && !batch.event.stopped
      ? 100
      : percentage(settled + activeFraction, batch.selected.length)

  const chooseSource = async () => {
    setLoadingSource(true)
    try {
      const source = await call(commands.browseBatchSource)
      if (!source) return
      const selected = source.chapters
        .filter((chapter) => !chapter.error)
        .map((chapter) => chapter.path)
      updateBatch({
        source,
        output: source.default_output,
        selected,
        event: null,
        report: null,
        statuses: Object.fromEntries(selected.map((path) => [path, 'idle'])),
      })
    } finally {
      setLoadingSource(false)
    }
  }

  const chooseOutput = async () => {
    setLoadingOutput(true)
    try {
      const output = await call(commands.browseBatchOutput)
      if (output) updateBatch({ output })
    } finally {
      setLoadingOutput(false)
    }
  }

  const toggleChapter = (path: string) => {
    if (batch.running) return
    updateBatch({
      selected: selected.has(path)
        ? batch.selected.filter((selectedPath) => selectedPath !== path)
        : [...batch.selected, path],
      report: null,
    })
  }

  const start = async () => {
    if (batch.selected.length === 0 || !batch.output || batch.running) return
    const paths = [...batch.selected]
    const statuses = Object.fromEntries(paths.map((path) => [path, 'idle' as const]))
    updateBatch({ running: true, event: null, report: null, statuses })
    const channel = new Channel<BatchEvent>()
    channel.onmessage = (event) => receiveBatchEvent(event, paths)
    try {
      const report = await call(commands.processBatch, paths, batch.output, 95, false, channel)
      updateBatch({ report })
    } finally {
      updateBatch({ running: false })
    }
  }

  return (
    <section className='flex min-h-0 flex-1 bg-[var(--surface-sidebar)] text-foreground'>
      <nav className='flex w-64 shrink-0 flex-col bg-[var(--surface-sidebar)] px-3 py-4'>
        <Button
          type='button'
          variant='ghost'
          className='mb-5 h-9 justify-start gap-2 rounded-lg px-2 text-[12px] text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground'
          onClick={() => setBatchOpen(false)}
        >
          <ArrowLeft className='size-4' /> {t('batch.backToEditor')}
        </Button>
        <p className='mb-2 px-2 text-[10px] font-semibold tracking-[0.14em] text-muted-foreground uppercase'>
          {t('batch.title')}
        </p>
        <p className='px-2 text-[11px] leading-5 text-muted-foreground'>{t('batch.description')}</p>
        <div className='mt-6 grid gap-2'>
          <Button
            type='button'
            variant='outline'
            className='h-10 justify-start gap-2 rounded-lg bg-foreground/[0.02] px-3 text-[11px]'
            disabled={batch.running || loadingSource}
            onClick={() => void chooseSource().catch(() => undefined)}
          >
            {loadingSource ? (
              <LoaderCircle className='size-4 animate-spin' />
            ) : (
              <FolderInput className='size-4' />
            )}
            {batch.source ? t('batch.changeSource') : t('batch.chooseSource')}
          </Button>
          <Button
            type='button'
            variant='outline'
            className='h-10 justify-start gap-2 rounded-lg bg-foreground/[0.02] px-3 text-[11px]'
            disabled={!batch.source || batch.running || loadingOutput}
            onClick={() => void chooseOutput().catch(() => undefined)}
          >
            {loadingOutput ? (
              <LoaderCircle className='size-4 animate-spin' />
            ) : (
              <FolderOutput className='size-4' />
            )}
            {t('batch.chooseOutput')}
          </Button>
        </div>
        {batch.source && (
          <div className='mt-5 grid gap-4 px-2'>
            <PathSummary label={t('batch.source')} path={batch.source.path} />
            <PathSummary label={t('batch.output')} path={batch.output} />
          </div>
        )}
        <div className='mt-auto rounded-xl border border-border/70 bg-foreground/[0.025] p-3'>
          <p className='text-[10px] font-semibold tracking-[0.1em] text-muted-foreground uppercase'>
            {t('batch.exportMode')}
          </p>
          <p className='mt-1.5 text-[11px] font-medium'>{t('batch.compactJpeg')}</p>
          <p className='mt-1 text-[10px] leading-4 text-muted-foreground'>
            {t('batch.exportHint')}
          </p>
        </div>
      </nav>

      <main className='relative z-10 flex min-w-0 flex-1 flex-col overflow-hidden rounded-tl-2xl bg-[var(--surface-canvas)] shadow-[var(--shadow-content)]'>
        <header className='flex h-14 shrink-0 items-center gap-4 border-b border-border/80 px-7'>
          <div className='min-w-0 flex-1'>
            <h1 className='text-[13px] font-semibold tracking-[-0.02em]'>{t('batch.title')}</h1>
            {batch.source && (
              <p className='mt-0.5 truncate text-[10px] text-muted-foreground'>
                {t('batch.chapterCount', { count: batch.source.chapters.length })}
              </p>
            )}
          </div>
          {batch.source && (
            <>
              <Button
                type='button'
                variant='ghost'
                size='sm'
                className='h-8 text-[11px]'
                disabled={batch.running}
                onClick={() =>
                  updateBatch({ selected: validChapters.map((chapter) => chapter.path) })
                }
              >
                {t('batch.selectAll')}
              </Button>
              <Button
                type='button'
                variant='ghost'
                size='sm'
                className='h-8 text-[11px]'
                disabled={batch.running || batch.selected.length === 0}
                onClick={() => updateBatch({ selected: [] })}
              >
                {t('batch.clearSelection')}
              </Button>
              <Button
                type='button'
                size='sm'
                className='h-8 gap-2 rounded-lg px-4 text-[11px]'
                disabled={batch.running || selectedChapters.length === 0 || !batch.output}
                onClick={() => void start().catch(() => undefined)}
              >
                {batch.running ? (
                  <LoaderCircle className='size-3.5 animate-spin' />
                ) : (
                  <Play className='size-3.5 fill-current' />
                )}
                {batch.running
                  ? t('batch.processing')
                  : t('batch.startSelected', { count: selectedChapters.length })}
              </Button>
            </>
          )}
        </header>

        {!batch.source ? (
          <EmptyBatch onChoose={chooseSource} loading={loadingSource} />
        ) : (
          <ScrollArea className='min-h-0 flex-1'>
            <div className={cn('p-7', (batch.running || batch.report) && 'pb-48')}>
              <div className='grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-4'>
                {batch.source.chapters.map((chapter) => (
                  <ChapterCard
                    key={chapter.path}
                    chapter={chapter}
                    selected={selected.has(chapter.path)}
                    disabled={batch.running}
                    status={batch.statuses[chapter.path] ?? 'idle'}
                    onToggle={() => toggleChapter(chapter.path)}
                  />
                ))}
              </div>
            </div>
          </ScrollArea>
        )}

        {(batch.running || batch.report) && batch.source && (
          <div className='absolute inset-x-5 bottom-5 rounded-2xl border border-border/80 bg-[color-mix(in_oklab,var(--surface-sidebar)_94%,transparent)] p-4 shadow-2xl backdrop-blur-xl'>
            <div className='grid gap-4 lg:grid-cols-2'>
              <Progress value={currentPercent} className='gap-2'>
                <ProgressLabel className='min-w-0 truncate text-[11px]'>
                  {currentChapter?.name ?? t('batch.preparing')}
                </ProgressLabel>
                <span className='ml-auto text-[10px] text-muted-foreground tabular-nums'>
                  {current?.event === 'chapter_progress'
                    ? t('batch.pageProgress', {
                        completed: current.completed_pages,
                        total: current.total_pages,
                      })
                    : `${Math.round(currentPercent)}%`}
                </span>
              </Progress>
              <Progress value={overallPercent} className='gap-2'>
                <ProgressLabel className='text-[11px]'>{t('batch.overallProgress')}</ProgressLabel>
                <span className='ml-auto text-[10px] text-muted-foreground tabular-nums'>
                  {t('batch.chapterProgress', {
                    completed: settled,
                    total: batch.selected.length,
                    percent: Math.round(overallPercent),
                  })}
                </span>
              </Progress>
            </div>
            <div className='mt-3 flex items-center justify-between gap-4 border-t border-border/60 pt-3'>
              <p className='truncate text-[10px] text-muted-foreground'>
                {batch.event?.event === 'chapter_progress' && batch.event.stage
                  ? t('batch.runningStage', { stage: t(`phase.${batch.event.stage}`) })
                  : batch.report?.stopped
                    ? t('batch.stoppedSummary', {
                        completed: batch.report.completed,
                        total: batch.selected.length,
                      })
                    : batch.report
                      ? t('batch.summary', {
                          completed: batch.report.completed,
                          skipped: batch.report.skipped,
                          failed: batch.report.failures.length,
                        })
                      : t('batch.usingSavedSettings')}
              </p>
              {batch.running && (
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  className='h-8 shrink-0 gap-2 rounded-lg text-[11px]'
                  onClick={() => void call(commands.stopBatch).catch(() => undefined)}
                >
                  <Square className='size-3 fill-current' /> {t('batch.stop')}
                </Button>
              )}
            </div>
          </div>
        )}
      </main>
    </section>
  )
}

function EmptyBatch({ onChoose, loading }: { onChoose: () => Promise<void>; loading: boolean }) {
  const { t } = useTranslation()
  return (
    <div className='grid min-h-0 flex-1 place-items-center p-8'>
      <div className='flex max-w-sm flex-col items-center text-center'>
        <div className='grid size-14 place-items-center rounded-2xl border border-primary/20 bg-primary/[0.08] text-primary'>
          <Images className='size-6' />
        </div>
        <h2 className='mt-5 text-[15px] font-semibold'>{t('batch.emptyTitle')}</h2>
        <p className='mt-2 text-[11px] leading-5 text-muted-foreground'>
          {t('batch.emptyDescription')}
        </p>
        <Button
          type='button'
          className='mt-5 h-9 gap-2 rounded-lg px-4 text-[11px]'
          disabled={loading}
          onClick={() => void onChoose().catch(() => undefined)}
        >
          {loading ? (
            <LoaderCircle className='size-4 animate-spin' />
          ) : (
            <FolderOpen className='size-4' />
          )}
          {t('batch.chooseSource')}
        </Button>
      </div>
    </div>
  )
}

function ChapterCard({
  chapter,
  selected,
  disabled,
  status,
  onToggle,
}: {
  chapter: BatchChapter
  selected: boolean
  disabled: boolean
  status: BatchChapterStatus
  onToggle: () => void
}) {
  const { t } = useTranslation()
  const invalid = Boolean(chapter.error)
  return (
    <button
      type='button'
      role='checkbox'
      aria-checked={selected}
      aria-label={chapter.name}
      disabled={disabled || invalid}
      className={cn(
        'group overflow-hidden rounded-xl border bg-[var(--surface-sidebar)] text-left transition-all disabled:cursor-not-allowed',
        selected
          ? 'border-primary/70 shadow-[0_0_0_1px_color-mix(in_oklab,var(--primary)_35%,transparent),0_12px_30px_rgba(0,0,0,0.2)]'
          : 'border-border/70 hover:-translate-y-0.5 hover:border-foreground/20 hover:shadow-xl',
        invalid && 'opacity-65',
      )}
      onClick={onToggle}
    >
      <div className='relative aspect-[4/3] overflow-hidden bg-foreground/[0.04]'>
        {chapter.thumbnail ? (
          <Image
            src={chapter.thumbnail}
            alt=''
            fill
            unoptimized
            className='object-cover transition-transform duration-300 group-hover:scale-[1.025]'
          />
        ) : (
          <div className='grid h-full place-items-center text-muted-foreground'>
            {invalid ? <TriangleAlert className='size-6' /> : <Images className='size-6' />}
          </div>
        )}
        <span
          className={cn(
            'absolute top-2 left-2 grid size-5 place-items-center rounded-md border shadow-sm backdrop-blur-md',
            selected
              ? 'border-primary bg-primary text-primary-foreground'
              : 'border-white/30 bg-black/40 text-transparent',
          )}
        >
          <Check className='size-3.5' />
        </span>
        <Badge className='absolute top-2 right-2 border-white/10 bg-black/55 px-1.5 py-0.5 text-[9px] text-white backdrop-blur-md'>
          CBZ
        </Badge>
        {status !== 'idle' && (
          <Badge
            className={cn(
              'absolute right-2 bottom-2 px-1.5 py-0.5 text-[9px]',
              status === 'failed'
                ? 'bg-destructive text-white'
                : status === 'running'
                  ? 'bg-primary text-primary-foreground'
                  : 'bg-emerald-600 text-white',
            )}
          >
            {t(`batch.status.${status}`)}
          </Badge>
        )}
      </div>
      <div className='p-3'>
        <p className='line-clamp-2 min-h-8 text-[11px] leading-4 font-medium' title={chapter.name}>
          {chapter.name}
        </p>
        <p className='mt-1.5 text-[9px] text-muted-foreground'>
          {chapter.error
            ? t('batch.unavailable')
            : t('batch.cardMeta', { size: formatBytes(chapter.size), pages: chapter.pages })}
        </p>
      </div>
    </button>
  )
}

function PathSummary({ label, path }: { label: string; path: string }) {
  return (
    <div className='min-w-0'>
      <p className='text-[9px] font-semibold tracking-[0.12em] text-muted-foreground uppercase'>
        {label}
      </p>
      <p className='mt-1 truncate text-[10px] text-foreground/85' title={path}>
        {path}
      </p>
    </div>
  )
}

function receiveBatchEvent(event: BatchEvent, selected: string[]) {
  const state = useKoharuStore.getState()
  const statuses = { ...state.batch.statuses }
  if ('index' in event) {
    const path = selected[event.index]
    if (path) {
      if (event.event === 'chapter_started' || event.event === 'chapter_progress') {
        statuses[path] = 'running'
      } else if (event.event === 'chapter_finished') {
        statuses[path] = 'completed'
      } else if (event.event === 'chapter_skipped') {
        statuses[path] = 'skipped'
      } else if (event.event === 'chapter_failed') {
        statuses[path] = 'failed'
      }
    }
  }
  state.updateBatch({ event, statuses })
}

function currentProgress(event: BatchEvent | null) {
  if (!event || !('index' in event)) return null
  return event
}

function percentage(value: number, total: number): number {
  if (total <= 0) return 0
  return Math.min(100, Math.max(0, (value / total) * 100))
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
