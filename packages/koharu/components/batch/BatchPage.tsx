'use client'

import {
  ArrowLeft,
  CheckCircle2,
  FileArchive,
  FolderInput,
  FolderOutput,
  LoaderCircle,
  Play,
  Settings,
  Square,
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
  const statuses = useBatchStore((s) => s.statuses)
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
      <div className='mx-auto flex min-h-0 w-full max-w-[1120px] flex-1 flex-col px-4 py-4 sm:px-8 sm:py-5'>
        <header className='mb-4 flex shrink-0 items-center justify-between gap-3'>
          <div className='flex items-center gap-3'>
            <Button
              variant='ghost'
              size='icon-sm'
              aria-label={t('common.back')}
              onClick={() => setWorkspace('project')}
            >
              <ArrowLeft className='size-4' />
            </Button>
            <h1 className='text-lg font-semibold tracking-tight'>{t('batch.tab')}</h1>
          </div>
          <div className='flex items-center gap-1' role='group' aria-label={t('batch.folders')}>
            <Button
              variant='ghost'
              size='icon-sm'
              aria-label={t('batch.chooseSource')}
              title={`${t('batch.source')}${source ? `\n${source.path}` : ''}`}
              disabled={!!busy || running}
              className={cn(source && 'text-primary')}
              onClick={() => void browseSource().catch(() => undefined)}
            >
              {busy === 'source' ? (
                <LoaderCircle className='size-4 animate-spin' />
              ) : (
                <FolderInput className='size-4' />
              )}
            </Button>
            <Button
              variant='ghost'
              size='icon-sm'
              aria-label={t('batch.chooseOutput')}
              title={`${t('batch.output')}${output ? `\n${output}` : ''}`}
              disabled={!!busy || running || !source}
              onClick={() => void browseOutput().catch(() => undefined)}
            >
              {busy === 'output' ? (
                <LoaderCircle className='size-4 animate-spin' />
              ) : (
                <FolderOutput className='size-4' />
              )}
            </Button>
            <span className='mx-2 h-4 w-px bg-border' aria-hidden='true' />
            <Button
              variant='ghost'
              size='icon-sm'
              aria-label={t('menu.settings')}
              title={t('menu.settings')}
              disabled={running}
              onClick={() => setWorkspace('settings')}
            >
              <Settings className='size-4' />
            </Button>
          </div>
        </header>
        <div className='mb-4 flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/70 pb-3'>
          <span className='text-[11px] text-muted-foreground tabular-nums'>
            {t('batch.selected', { count: selected.length, total: chapters.length })}
          </span>
          <div className='flex items-center gap-1'>
            <Button
              variant='ghost'
              size='sm'
              className='h-7 text-[11px]'
              disabled={!!busy || running || !pending.length}
              onClick={() => select(pending.map((c) => c.path))}
            >
              {t('batch.selectAll')}
            </Button>
            <Button
              variant='ghost'
              size='sm'
              className='h-7 text-[11px]'
              disabled={!!busy || running || !selected.length}
              onClick={() => select([])}
            >
              {t('batch.clear')}
            </Button>
            {running ? (
              <Button
                variant='outline'
                size='sm'
                className='ml-2 h-7 gap-1.5 text-[11px]'
                disabled={stopping}
                onClick={() => void stopBatch().catch(() => undefined)}
              >
                {stopping ? (
                  <LoaderCircle className='size-3 animate-spin' />
                ) : (
                  <Square className='size-3' />
                )}
                {t(stopping ? 'batch.stopping' : 'activity.stop')}
              </Button>
            ) : (
              <Button
                size='sm'
                className='ml-2 h-7 gap-1.5 text-[11px] text-[#1f080c]'
                disabled={!!busy || !selected.length || !output}
                onClick={() => void startBatch().catch(() => undefined)}
              >
                <Play className='size-3' />
                {t('batch.translate', { count: selected.length })}
              </Button>
            )}
          </div>
        </div>
        <section className='min-h-0 flex-1 overflow-y-auto' aria-label={t('batch.chapters')}>
          {!chapters.length ? (
            <div className='grid h-full place-items-center p-8 text-center'>
              <div className='max-w-xs'>
                <FileArchive
                  className='mx-auto mb-3 size-8 text-muted-foreground/50'
                  aria-hidden='true'
                />
                <p className='text-xs font-medium'>
                  {t(source ? 'batch.emptyFolder' : 'batch.emptyTitle')}
                </p>
              </div>
            </div>
          ) : (
            <div className='grid grid-cols-[repeat(auto-fill,minmax(140px,1fr))] gap-4 pb-3 sm:gap-5'>
              {chapters.map((chapter) => {
                const done = exported.includes(chapter.path)
                const status = statuses[chapter.path]
                return (
                  <label
                    key={chapter.path}
                    className={cn(
                      'group relative min-w-0 rounded-xl border bg-[var(--surface-panel)] p-2 transition-colors',
                      done
                        ? 'border-green-600/40 dark:border-green-400/35'
                        : selected.includes(chapter.path)
                          ? 'border-primary/70 bg-primary/[0.04]'
                          : 'border-border/80',
                      chapter.error && 'opacity-55',
                      !running &&
                        !chapter.error &&
                        !done &&
                        'cursor-pointer hover:border-primary/60',
                    )}
                  >
                    <span className='relative block aspect-[3/4] overflow-hidden rounded-lg bg-[var(--surface-well)]'>
                      {chapter.thumbnail ? (
                        <img
                          src={chapter.thumbnail}
                          alt=''
                          loading='lazy'
                          className='size-full object-cover'
                        />
                      ) : (
                        <span className='grid size-full place-items-center'>
                          <FileArchive className='size-7 text-muted-foreground/40' />
                        </span>
                      )}
                      <input
                        type='checkbox'
                        checked={selected.includes(chapter.path)}
                        disabled={!!busy || running || !!chapter.error || done}
                        aria-label={chapter.name}
                        className='absolute top-2 left-2 size-4 accent-[var(--primary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring'
                        onChange={(event) =>
                          select(
                            event.currentTarget.checked
                              ? [...selected, chapter.path]
                              : selected.filter((path) => path !== chapter.path),
                          )
                        }
                      />
                      {done && (
                        <span className='absolute top-2 right-2 rounded-full bg-[var(--surface-panel)] p-1 text-green-700 dark:text-green-300'>
                          <CheckCircle2 className='size-4' aria-hidden='true' />
                        </span>
                      )}
                    </span>
                    <span className='mt-2 block truncate text-xs font-medium' title={chapter.name}>
                      {chapter.name}
                    </span>
                    <span className='mt-1 flex min-w-0 items-center justify-between gap-2 text-[10px]'>
                      <span
                        className={cn(
                          'truncate',
                          done
                            ? 'text-green-700 dark:text-green-300'
                            : chapter.error || status === 'failed'
                              ? 'text-destructive'
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
                                ? `batch.${status}`
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
        </section>
        <BatchProgress />
      </div>
    </main>
  )
}
