'use client'

import {
  ArrowLeft,
  Check,
  Download,
  FileArchive,
  FolderOpen,
  LoaderCircle,
  Settings,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BatchProgress } from '@/components/batch/BatchProgress'
import { call } from '@/lib/backend'
import { startBatch, stopBatch, useBatchStore } from '@/lib/batch'
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
  const activePath = useBatchStore((s) => s.activePath)
  const statuses = useBatchStore((s) => s.statuses)
  const report = useBatchStore((s) => s.report)
  const setWorkspace = useKoharuStore((s) => s.setWorkspace)
  const [busy, setBusy] = useState<'source' | 'output' | null>(null)
  const chapters = source?.chapters ?? []
  const pending = chapters.filter((c) => !c.error && !exported.includes(c.path))

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
      <header className='flex shrink-0 flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b border-border/60 px-5 py-4 sm:px-8'>
        <div className='flex min-w-0 items-center gap-4'>
          <Button
            variant='ghost'
            size='icon'
            aria-label={t('common.back')}
            onClick={() => setWorkspace('project')}
            className='-ms-2 text-muted-foreground'
          >
            <ArrowLeft className='size-[18px]' strokeWidth={1.75} />
          </Button>
          <div className='min-w-0'>
            <h1 className='text-xl leading-7 font-semibold tracking-tight'>{t('batch.tab')}</h1>
            {source && (
              <p
                className='mt-0.5 max-w-64 truncate text-xs text-muted-foreground'
                title={source.path}
              >
                {source.path.split(/[\\/]/).filter(Boolean).pop()}
              </p>
            )}
          </div>
        </div>
        <div className='flex items-center gap-4'>
          <div className='flex items-center gap-1' role='group' aria-label={t('batch.folders')}>
            <Button
              variant='ghost'
              size='icon'
              aria-label={t('batch.chooseSource')}
              title={t('batch.source') + (source ? '\n' + source.path : '')}
              disabled={!!busy || running}
              onClick={() => void browseSource().catch(() => undefined)}
              className='text-foreground/70'
            >
              {busy === 'source' ? (
                <LoaderCircle className='size-[18px]' strokeWidth={1.75} />
              ) : (
                <FolderOpen className='size-[18px]' strokeWidth={1.75} />
              )}
            </Button>
            <Button
              variant='ghost'
              size='icon'
              aria-label={t('batch.chooseOutput')}
              title={t('batch.output') + (output ? '\n' + output : '')}
              disabled={!!busy || running || !source}
              onClick={() => void browseOutput().catch(() => undefined)}
              className='text-foreground/70'
            >
              {busy === 'output' ? (
                <LoaderCircle className='size-[18px]' strokeWidth={1.75} />
              ) : (
                <Download className='size-[18px]' strokeWidth={1.75} />
              )}
            </Button>
            <Button
              variant='ghost'
              size='icon'
              aria-label={t('menu.settings')}
              title={t('menu.settings')}
              disabled={running}
              onClick={() => setWorkspace('settings')}
              className='text-foreground/70'
            >
              <Settings className='size-[18px]' strokeWidth={1.75} />
            </Button>
          </div>
          <span className='h-5 w-px bg-border' aria-hidden='true' />
          {running ? (
            <Button
              variant='outline'
              className='min-w-28 text-xs'
              disabled={stopping}
              onClick={() => void stopBatch().catch(() => undefined)}
            >
              {t(stopping ? 'batch.stopping' : 'activity.stop')}
            </Button>
          ) : (
            <Button
              className='min-w-28 px-4 text-xs'
              disabled={!!busy || !selected.length || !output}
              onClick={() => void startBatch().catch(() => undefined)}
            >
              {t('batch.translate', { count: selected.length })}
            </Button>
          )}
        </div>
      </header>
      <div className='flex min-h-0 flex-1 flex-col min-[900px]:flex-row'>
        <section className='flex min-h-0 min-w-0 flex-1 flex-col' aria-label={t('batch.chapters')}>
          <div className='flex shrink-0 flex-wrap items-center justify-between gap-3 px-5 py-5 sm:px-8'>
            <span className='text-xs text-muted-foreground tabular-nums'>
              {t('batch.selected', { count: selected.length, total: chapters.length })}
            </span>
            <div className='flex items-center gap-2'>
              <Button
                variant='ghost'
                size='sm'
                className='text-xs text-muted-foreground'
                disabled={!!busy || running || !pending.length}
                onClick={() => select(pending.map((c) => c.path))}
              >
                {t('batch.selectAll')}
              </Button>
              <Button
                variant='ghost'
                size='sm'
                className='text-xs text-muted-foreground'
                disabled={!!busy || running || !selected.length}
                onClick={() => select([])}
              >
                {t('batch.clear')}
              </Button>
            </div>
          </div>
          {!running && report && (
            <p role='status' className='px-5 pb-5 text-xs text-muted-foreground sm:px-8'>
              {report.stopped && <span className='me-2 font-medium'>{t('batch.stopped')}</span>}
              {t('batch.summary', {
                completed: report.completed,
                skipped: report.skipped,
                failed: report.failures.length,
              })}
            </p>
          )}
          <div className='min-h-0 flex-1 overflow-y-auto px-5 pb-8 sm:px-8'>
            {!chapters.length ? (
              <div className='grid h-full min-h-48 place-items-center text-center'>
                <div className='max-w-xs py-12'>
                  <FolderOpen
                    className='mx-auto mb-5 size-10 text-muted-foreground/40'
                    strokeWidth={1}
                    aria-hidden='true'
                  />
                  <p className='text-base font-medium'>
                    {t(source ? 'batch.emptyFolder' : 'batch.emptyTitle')}
                  </p>
                  <Button
                    variant='secondary'
                    className='mt-5 text-xs'
                    disabled={!!busy}
                    onClick={() => void browseSource().catch(() => undefined)}
                  >
                    {t('batch.chooseSource')}
                  </Button>
                </div>
              </div>
            ) : (
              <div className='grid grid-cols-[repeat(auto-fill,minmax(min(100%,190px),1fr))] gap-x-6 gap-y-8 pt-1 pb-2'>
                {chapters.map((chapter) => {
                  const done = exported.includes(chapter.path)
                  const status = statuses[chapter.path]
                  return (
                    <label
                      key={chapter.path}
                      className={cn(
                        'group block min-w-0',
                        !running && !busy && !chapter.error && !done && 'cursor-pointer',
                      )}
                    >
                      <span
                        className={cn(
                          'relative block aspect-[3/4] rounded-lg bg-[var(--surface-well)] shadow-sm ring-1 ring-foreground/5',
                          (status === 'running' || status === 'exporting') &&
                            'ring-2 ring-primary ring-offset-4 ring-offset-[var(--surface-canvas)]',
                        )}
                      >
                        {chapter.thumbnail ? (
                          <img
                            src={chapter.thumbnail}
                            alt=''
                            loading='lazy'
                            className='size-full rounded-lg object-contain'
                          />
                        ) : (
                          <span className='grid size-full place-items-center'>
                            <FileArchive
                              className='size-9 text-muted-foreground/30'
                              strokeWidth={1.25}
                            />
                          </span>
                        )}
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
                        {!done && !chapter.error && (
                          <span
                            aria-hidden='true'
                            className='absolute top-3 left-3 grid size-5 place-items-center rounded-full border border-white/80 bg-black/25 text-primary-foreground shadow-sm peer-checked:border-primary peer-checked:bg-primary peer-focus-visible:outline-2 peer-focus-visible:outline-offset-4 peer-focus-visible:outline-ring [&>svg]:invisible peer-checked:[&>svg]:visible'
                          >
                            <Check className='size-3.5' strokeWidth={2.5} />
                          </span>
                        )}
                        {done && (
                          <span
                            aria-hidden='true'
                            className='absolute top-3 right-3 grid size-6 place-items-center rounded-full bg-green-700 text-white shadow-sm dark:bg-green-300 dark:text-green-950'
                          >
                            <Check className='size-4' strokeWidth={2} />
                          </span>
                        )}
                      </span>
                      <span
                        className='mt-3 line-clamp-2 block text-[13px] leading-5 font-medium break-words'
                        title={chapter.name}
                      >
                        {chapter.name.replace(/\.cbz$/i, '')}
                      </span>
                      <span className='mt-1 flex min-w-0 items-center justify-between gap-2 text-[11px]'>
                        <span
                          className={cn(
                            'truncate',
                            done
                              ? 'text-green-700 dark:text-green-300'
                              : chapter.error || status === 'failed'
                                ? 'text-destructive'
                                : status === 'running' || status === 'exporting'
                                  ? 'text-primary'
                                  : 'text-muted-foreground',
                          )}
                          title={chapter.error ?? undefined}
                        >
                          {chapter.error ??
                            t(
                              done
                                ? status === 'translated'
                                  ? 'batch.translated'
                                  : 'batch.alreadyTranslated'
                                : status
                                  ? 'batch.' + status
                                  : 'batch.ready',
                            )}
                        </span>
                        <span className='shrink-0 text-muted-foreground tabular-nums'>
                          {t('batch.pages', { count: chapter.pages })}
                        </span>
                      </span>
                    </label>
                  )
                })}
              </div>
            )}
          </div>
        </section>
        <BatchProgress key={running ? activePath : 'idle'} />
      </div>
    </main>
  )
}
