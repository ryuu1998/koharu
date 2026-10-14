'use client'

import {
  ArrowLeft,
  Check,
  Download,
  FileArchive,
  FolderOpen,
  LoaderCircle,
  Settings,
  Pause,
  Play,
  Upload,
  TriangleAlert,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BatchQueue } from '@/components/batch/BatchQueue'
import { BatchViewControls } from '@/components/batch/BatchViewControls'
import { call } from '@/lib/backend'
import {
  pauseBatch,
  resumeBatch,
  sortBatchChapters,
  startBatch,
  stopBatch,
  useBatchStore,
} from '@/lib/batch'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { cn } from '@koharu/ui/lib/utils'

export function BatchPage() {
  const { t } = useTranslation()
  const source = useBatchStore((s) => s.source)
  const output = useBatchStore((s) => s.output)
  const selected = useBatchStore((s) => s.selected)
  const exported = useBatchStore((s) => s.exported)
  const setSource = useBatchStore((s) => s.setSource)
  const setOutput = useBatchStore((s) => s.setOutput)
  const select = useBatchStore((s) => s.select)
  const running = useBatchStore((s) => s.running)
  const stopping = useBatchStore((s) => s.stopping)
  const pauseState = useBatchStore((s) => s.pauseState)
  const cardSize = useBatchStore((s) => s.cardSize)
  const cardLayout = useBatchStore((s) => s.cardLayout)
  const sortBy = useBatchStore((s) => s.sortBy)
  const sortDescending = useBatchStore((s) => s.sortDescending)
  const statuses = useBatchStore((s) => s.statuses)
  const report = useBatchStore((s) => s.report)
  const setWorkspace = useKoharuStore((s) => s.setWorkspace)
  const [busy, setBusy] = useState<'source' | 'output' | null>(null)
  const chapters = sortBatchChapters(source?.chapters ?? [], sortBy, sortDescending)

  const browseSource = async () => {
    if (busy || running) return
    setBusy('source')
    try {
      const source = await call(commands.browseBatchSource)
      if (source) setSource(source)
    } finally {
      setBusy(null)
    }
  }
  const browseOutput = async () => {
    if (busy || running) return
    setBusy('output')
    try {
      const output = await call(commands.browseBatchOutput)
      if (output) await setOutput(output)
    } finally {
      setBusy(null)
    }
  }

  return (
    <main className='flex min-h-0 flex-1 flex-col overflow-hidden bg-[var(--surface-canvas)]'>
      <header className='flex min-h-12 shrink-0 flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-border/70 px-4 py-2 sm:px-5'>
        <div className='flex min-w-0 items-center gap-2'>
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={t('common.back')}
            onClick={() => setWorkspace('project')}
            className='-ms-1 text-muted-foreground'
          >
            <ArrowLeft className='size-4' strokeWidth={1.75} />
          </Button>
          <h1 className='shrink-0 text-sm font-semibold'>{t('batch.tab')}</h1>
          {source && (
            <>
              <span className='mx-1 hidden text-border min-[700px]:inline' aria-hidden='true'>
                /
              </span>
              <span
                className='hidden max-w-[28vw] truncate text-[11px] text-muted-foreground min-[700px]:inline'
                title={source.path}
              >
                {source.path.split(/[\\/]/).filter(Boolean).pop()}
              </span>
            </>
          )}
        </div>
        {running ? (
          <div className='flex items-center gap-2'>
            <Button
              variant='outline'
              size='sm'
              className='gap-1.5 text-[11px]'
              disabled={stopping || pauseState === 'pausing'}
              onClick={() =>
                void (pauseState === 'paused' ? resumeBatch() : pauseBatch()).catch(() => undefined)
              }
            >
              {pauseState === 'paused' ? (
                <Play className='size-3.5' />
              ) : (
                <Pause className='size-3.5' />
              )}
              {t(
                'batch.' +
                  (pauseState === 'paused'
                    ? 'resume'
                    : pauseState === 'pausing'
                      ? 'pausing'
                      : 'pause'),
              )}
            </Button>
            <Button
              variant='outline'
              size='sm'
              className='min-w-24 text-[11px]'
              disabled={stopping}
              onClick={() => void stopBatch().catch(() => undefined)}
            >
              {t(stopping ? 'batch.stopping' : 'activity.stop')}
            </Button>
          </div>
        ) : (
          <Button
            size='sm'
            className='px-3 text-[11px]'
            disabled={!!busy || !selected.length || !output}
            onClick={() => void startBatch().catch(() => undefined)}
          >
            {t('batch.translate', { count: selected.length })}
          </Button>
        )}
      </header>
      <div
        className='flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-border/70 bg-[var(--surface-toolbar)] px-4 py-2 sm:px-5'
        role='toolbar'
        aria-label={t('batch.toolbar')}
      >
        <div className='flex items-center gap-0.5' role='group' aria-label={t('batch.folders')}>
          <Button
            variant='ghost'
            size='sm'
            aria-label={t('batch.chooseSource')}
            title={t('batch.source') + (source ? '\n' + source.path : '')}
            disabled={!!busy || running}
            onClick={() => void browseSource().catch(() => undefined)}
            className='gap-1.5 text-[11px] text-foreground/80'
          >
            {busy === 'source' ? (
              <LoaderCircle className='size-4' strokeWidth={1.75} />
            ) : (
              <Download className='size-4' strokeWidth={1.75} />
            )}
            {t('batch.source')}
          </Button>
          <Button
            variant='ghost'
            size='sm'
            aria-label={t('batch.chooseOutput')}
            title={t('batch.output') + (output ? '\n' + output : '')}
            disabled={!!busy || running || !source}
            onClick={() => void browseOutput().catch(() => undefined)}
            className='gap-1.5 text-[11px] text-foreground/80'
          >
            {busy === 'output' ? (
              <LoaderCircle className='size-4' strokeWidth={1.75} />
            ) : (
              <Upload className='size-4' strokeWidth={1.75} />
            )}
            {t('batch.output')}
          </Button>
        </div>
        <span className='flex-1' />
        <BatchViewControls busy={!!busy} />
        <Button
          variant='ghost'
          size='icon-sm'
          aria-label={t('menu.settings')}
          title={t('menu.settings')}
          disabled={running}
          onClick={() => setWorkspace('settings')}
          className='text-muted-foreground'
        >
          <Settings className='size-4' strokeWidth={1.75} />
        </Button>
      </div>
      <div className='flex min-h-0 flex-1 flex-col min-[800px]:flex-row'>
        <section className='flex min-h-0 min-w-0 flex-1 flex-col' aria-label={t('batch.chapters')}>
          <div className='flex min-h-11 shrink-0 items-center justify-between gap-3 px-4 sm:px-5'>
            <span className='text-[11px] text-muted-foreground tabular-nums'>
              {t('batch.selected', { count: selected.length, total: chapters.length })}
            </span>
          </div>
          {!running && report && (
            <p role='status' className='px-4 pb-3 text-[11px] text-muted-foreground sm:px-5'>
              {report.stopped && <span className='me-2 font-medium'>{t('batch.stopped')}</span>}
              {t('batch.summary', {
                completed: report.completed,
                skipped: report.skipped,
                failed: report.failures.length,
              })}
            </p>
          )}
          <div className='min-h-0 flex-1 overflow-y-auto px-4 pb-5 sm:px-5'>
            {!chapters.length ? (
              <div className='grid h-full min-h-48 place-items-center'>
                <div className='text-center'>
                  <p className='text-[13px] font-medium'>
                    {t(source ? 'batch.emptyFolder' : 'batch.emptyTitle')}
                  </p>
                  <Button
                    variant='outline'
                    size='sm'
                    className='mt-3 gap-2 text-[11px]'
                    disabled={!!busy}
                    onClick={() => void browseSource().catch(() => undefined)}
                  >
                    <FolderOpen className='size-3.5' />
                    {t('batch.chooseSource')}
                  </Button>
                </div>
              </div>
            ) : (
              <div
                className='grid gap-x-3 gap-y-3'
                style={{ gridTemplateColumns: `repeat(auto-fill, ${cardSize + 16}px)` }}
              >
                {chapters.map((chapter) => {
                  const done = exported.includes(chapter.path)
                  const status = statuses[chapter.path]
                  const active = status === 'running' || status === 'exporting'
                  const failed = !!chapter.error || status === 'failed'
                  const caption = done
                    ? status === 'translated'
                      ? 'batch.translated'
                      : 'batch.alreadyTranslated'
                    : failed
                      ? 'batch.failed'
                      : status
                        ? 'batch.' + status
                        : 'batch.ready'
                  return (
                    <label
                      key={chapter.path}
                      className={cn(
                        'group relative flex min-w-0 flex-col rounded-lg p-2',
                        !running &&
                          !busy &&
                          !chapter.error &&
                          !done &&
                          'cursor-pointer hover:bg-muted/50',
                        selected.includes(chapter.path) &&
                          !done &&
                          'bg-primary/[0.035] ring-1 ring-primary/15 ring-inset',
                        active && 'bg-primary/[0.06] ring-primary/35',
                        done && 'bg-green-600/[0.055] ring-1 ring-green-600/25 ring-inset',
                      )}
                      title={[
                        chapter.name,
                        t(caption),
                        t('batch.pages', { count: chapter.pages }),
                        `${(chapter.size / 1048576).toFixed(1)} MB`,
                      ].join(' · ')}
                      style={{ width: cardSize + 16 }}
                    >
                      <span
                        className='relative grid shrink-0 place-items-center overflow-hidden rounded bg-[var(--surface-well)]'
                        style={{ width: cardSize, height: (cardSize * 4) / 3 }}
                      >
                        {chapter.thumbnail ? (
                          <img
                            src={chapter.thumbnail}
                            alt=''
                            loading='lazy'
                            className='object-contain'
                            style={{ width: cardSize, height: (cardSize * 4) / 3 }}
                          />
                        ) : (
                          <FileArchive
                            className='size-6 text-muted-foreground/35'
                            strokeWidth={1.25}
                          />
                        )}
                        <span
                          className={cn(
                            'absolute top-1 left-1 grid size-5 place-items-center rounded',
                            !chapter.error && 'bg-background/95',
                          )}
                        >
                          <input
                            type='checkbox'
                            checked={selected.includes(chapter.path)}
                            disabled={!!busy || running || !!chapter.error || done}
                            aria-label={chapter.name}
                            className='peer sr-only'
                            onChange={(event) =>
                              select(
                                event.currentTarget.checked
                                  ? [...selected, chapter.path]
                                  : selected.filter((path) => path !== chapter.path),
                              )
                            }
                          />
                          {done ? (
                            <Check
                              className='size-4 text-green-700 dark:text-green-300'
                              strokeWidth={2}
                              aria-hidden='true'
                            />
                          ) : (
                            !chapter.error && (
                              <span
                                aria-hidden='true'
                                className='grid size-3.5 place-items-center rounded-[3px] border border-input text-primary-foreground peer-checked:border-primary peer-checked:bg-primary peer-focus-visible:outline-2 peer-focus-visible:outline-offset-3 peer-focus-visible:outline-ring [&>svg]:invisible peer-checked:[&>svg]:visible'
                              >
                                <Check className='size-2.5' strokeWidth={2.5} />
                              </span>
                            )
                          )}
                        </span>
                      </span>
                      <span
                        className={cn(
                          'line-clamp-2 text-[11px] leading-4 font-medium break-words',
                          cardLayout === 'overlay'
                            ? 'pointer-events-none absolute inset-x-2 bottom-2 rounded-b bg-gradient-to-t from-black/90 via-black/70 to-transparent px-1.5 pt-5 pb-1.5 text-white'
                            : 'mt-1.5 h-8',
                        )}
                        title={chapter.name}
                      >
                        {chapter.name.replace(/\.cbz$/i, '')}
                      </span>
                      <span
                        className='absolute top-3 right-3 rounded bg-black/70 px-1 py-0.5 text-[10px] leading-3 text-white tabular-nums'
                        aria-label={t('batch.pages', { count: chapter.pages })}
                      >
                        {chapter.pages}P
                      </span>
                      {(done || failed) && (
                        <span className='sr-only' title={chapter.error ?? t(caption)}>
                          {t(done ? 'batch.translated' : caption)}
                        </span>
                      )}
                      {failed && (
                        <TriangleAlert
                          className='absolute top-3 left-3 size-4 text-destructive'
                          aria-hidden='true'
                        />
                      )}
                    </label>
                  )
                })}
              </div>
            )}
          </div>
        </section>
        <BatchQueue />
      </div>
    </main>
  )
}
