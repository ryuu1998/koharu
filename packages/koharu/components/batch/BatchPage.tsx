'use client'

import { ArrowLeft, FileArchive, FolderOpen, LoaderCircle, Settings } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { useBatchStore } from '@/lib/batch'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { cn } from '@koharu/ui/lib/utils'

export function BatchPage() {
  const { t } = useTranslation()
  const { source, output, selected, setSource, setOutput, select } = useBatchStore()
  const setWorkspace = useKoharuStore((state) => state.setWorkspace)
  const [busy, setBusy] = useState<'source' | 'output' | null>(null)
  const chapters = source?.chapters ?? []
  const healthy = chapters.filter((chapter) => !chapter.error)

  const browseSource = async () => {
    if (busy) return
    setBusy('source')
    try {
      const source = await call(commands.browseBatchSource)
      if (source) setSource(source)
    } finally {
      setBusy(null)
    }
  }
  const browseOutput = async () => {
    if (busy) return
    setBusy('output')
    try {
      const output = await call(commands.browseBatchOutput)
      if (output) setOutput(output)
    } finally {
      setBusy(null)
    }
  }

  return (
    <main className='flex min-h-0 flex-1 flex-col overflow-hidden bg-[var(--surface-canvas)]'>
      <div className='mx-auto flex min-h-0 w-full max-w-[1120px] flex-1 flex-col px-6 py-5 sm:px-8'>
        <header className='mb-5 flex shrink-0 items-center justify-between gap-4'>
          <div className='flex items-center gap-3'>
            <Button
              variant='ghost'
              size='icon-sm'
              aria-label={t('common.back')}
              onClick={() => setWorkspace('project')}
            >
              <ArrowLeft className='size-4' />
            </Button>
            <div>
              <h1 className='text-lg font-semibold tracking-tight'>{t('batch.title')}</h1>
              <p className='mt-0.5 text-[11px] text-muted-foreground'>{t('batch.description')}</p>
            </div>
          </div>
          <Button
            variant='ghost'
            size='sm'
            className='gap-1.5 text-xs text-muted-foreground'
            onClick={() => setWorkspace('settings')}
          >
            <Settings className='size-3.5' />
            {t('menu.settings')}
          </Button>
        </header>

        <section
          className='mb-5 grid shrink-0 grid-cols-1 gap-3 sm:grid-cols-2'
          aria-label={t('batch.folders')}
        >
          {[
            {
              label: t('batch.source'),
              path: source?.path,
              action: browseSource,
              pending: busy === 'source',
              disabled: !!busy,
              title: t('batch.chooseSource'),
            },
            {
              label: t('batch.output'),
              path: output,
              action: browseOutput,
              pending: busy === 'output',
              disabled: !!busy || !source,
              title: t('batch.chooseOutput'),
            },
          ].map((folder) => (
            <button
              key={folder.label}
              type='button'
              disabled={folder.disabled}
              onClick={() => void folder.action().catch(() => undefined)}
              aria-label={folder.title}
              className='group flex min-w-0 items-center gap-3 rounded-xl border border-border/80 bg-[var(--surface-panel)] px-4 py-3 text-left hover:border-primary/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50'
            >
              <span className='grid size-9 shrink-0 place-items-center rounded-lg bg-[var(--surface-well)] text-muted-foreground'>
                {folder.pending ? (
                  <LoaderCircle className='size-4 animate-spin' aria-hidden='true' />
                ) : (
                  <FolderOpen className='size-4' aria-hidden='true' />
                )}
              </span>
              <span className='min-w-0 flex-1'>
                <span className='block text-[11px] font-medium'>{folder.label}</span>
                <span
                  className='mt-0.5 block truncate text-[11px] text-muted-foreground'
                  title={folder.path ?? undefined}
                >
                  {folder.path ?? folder.title}
                </span>
              </span>
              <span className='shrink-0 text-[11px] text-muted-foreground group-hover:text-foreground'>
                {t('batch.choose')}
              </span>
            </button>
          ))}
        </section>

        <section
          className='flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border/80 bg-[var(--surface-panel)]'
          aria-label={t('batch.chapters')}
        >
          <div className='flex min-h-12 shrink-0 items-center justify-between gap-3 border-b border-border/70 px-4'>
            <div className='flex items-baseline gap-2'>
              <h2 className='text-xs font-semibold'>{t('batch.chapters')}</h2>
              <span className='text-[11px] text-muted-foreground tabular-nums'>
                {t('batch.selected', { count: selected.length, total: chapters.length })}
              </span>
            </div>
            <div className='flex gap-1'>
              <Button
                variant='ghost'
                size='sm'
                className='h-7 text-[11px]'
                disabled={!!busy || !healthy.length}
                onClick={() => select(healthy.map((chapter) => chapter.path))}
              >
                {t('batch.selectAll')}
              </Button>
              <Button
                variant='ghost'
                size='sm'
                className='h-7 text-[11px]'
                disabled={!!busy || !selected.length}
                onClick={() => select([])}
              >
                {t('batch.clear')}
              </Button>
            </div>
          </div>
          {chapters.length === 0 ? (
            <div className='grid flex-1 place-items-center p-8 text-center'>
              <div className='max-w-xs'>
                <FileArchive
                  className='mx-auto mb-3 size-8 text-muted-foreground/50'
                  aria-hidden='true'
                />
                <p className='text-xs font-medium'>
                  {t(source ? 'batch.emptyFolder' : 'batch.emptyTitle')}
                </p>
                <p className='mt-1.5 text-[11px] leading-5 text-muted-foreground'>
                  {t(source ? 'batch.emptyFolderHint' : 'batch.emptyHint')}
                </p>
              </div>
            </div>
          ) : (
            <div className='min-h-0 flex-1 overflow-y-auto'>
              {chapters.map((chapter) => (
                <label
                  key={chapter.path}
                  className={cn(
                    'flex min-h-20 items-center gap-4 border-b border-border/50 px-4 py-3 last:border-b-0',
                    chapter.error ? 'opacity-60' : 'cursor-pointer hover:bg-primary/[0.025]',
                    selected.includes(chapter.path) && 'bg-primary/[0.04]',
                  )}
                >
                  <input
                    type='checkbox'
                    checked={selected.includes(chapter.path)}
                    disabled={!!busy || !!chapter.error}
                    aria-label={chapter.name}
                    className='size-3.5 shrink-0 accent-[var(--primary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring'
                    onChange={(event) =>
                      select(
                        event.currentTarget.checked
                          ? [...selected, chapter.path]
                          : selected.filter((path) => path !== chapter.path),
                      )
                    }
                  />
                  <span className='grid h-14 w-10 shrink-0 place-items-center overflow-hidden rounded-md bg-[var(--surface-well)]'>
                    {chapter.thumbnail ? (
                      <img
                        src={chapter.thumbnail}
                        alt=''
                        loading='lazy'
                        className='size-full object-cover'
                      />
                    ) : (
                      <FileArchive className='size-4 text-muted-foreground/60' aria-hidden='true' />
                    )}
                  </span>
                  <span className='min-w-0 flex-1'>
                    <span className='block truncate text-xs font-medium' title={chapter.name}>
                      {chapter.name}
                    </span>
                    <span
                      className={cn(
                        'mt-1 block truncate text-[11px]',
                        chapter.error ? 'text-destructive' : 'text-muted-foreground',
                      )}
                      title={chapter.error ?? undefined}
                    >
                      {chapter.error ?? t('batch.ready')}
                    </span>
                  </span>
                  <span className='w-20 text-right text-[11px] text-muted-foreground tabular-nums'>
                    {t('batch.pages', { count: chapter.pages })}
                  </span>
                  <span className='w-16 text-right text-[11px] text-muted-foreground tabular-nums'>
                    {(chapter.size / 1048576).toLocaleString(undefined, {
                      maximumFractionDigits: 1,
                    })}{' '}
                    MB
                  </span>
                </label>
              ))}
            </div>
          )}
        </section>
        <p className='mt-3 shrink-0 text-[10px] text-muted-foreground'>{t('batch.exportHint')}</p>
      </div>
    </main>
  )
}
