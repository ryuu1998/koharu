'use client'

import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { commands, type BatchSource } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from '@koharu/ui/components/dialog'
import { cn } from '@koharu/ui/lib/utils'

export function BatchImportDialog({
  source,
  currentOutput,
  onClose,
  onImport,
}: {
  source: BatchSource
  currentOutput: string
  onClose: () => void
  onImport: (output: string) => Promise<void>
}) {
  const { t } = useTranslation()
  const [destination, setDestination] = useState(currentOutput)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const count = source.chapters.filter((chapter) => !chapter.error).length
  const choices = [
    { label: t('batch.useCurrentOutput'), path: currentOutput },
    { label: t('batch.useImportOutput'), path: source.default_output },
  ].filter(
    (choice, index, choices) => choices.findIndex((other) => other.path === choice.path) === index,
  )

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose()
      }}
    >
      <DialogContent showCloseButton={!busy}>
        <DialogTitle>{t('batch.addArchives')}</DialogTitle>
        <DialogDescription className='truncate text-xs' title={source.path}>
          {source.path}
        </DialogDescription>
        <fieldset className='grid gap-2' disabled={busy}>
          <legend className='mb-2 text-xs font-medium'>{t('batch.output')}</legend>
          {choices.map(({ label, path }) => (
            <label
              key={path}
              className={cn(
                'flex cursor-pointer items-start gap-3 rounded-lg border p-3',
                destination === path ? 'border-primary/40 bg-primary/[0.04]' : 'border-border',
              )}
            >
              <input
                type='radio'
                name='batch-import-destination'
                checked={destination === path}
                onChange={() => setDestination(path)}
                className='mt-0.5 accent-primary'
              />
              <span className='min-w-0 text-xs'>
                <span className='font-medium'>{label}</span>
                <span
                  className='mt-1 block truncate text-[10px] text-muted-foreground'
                  title={path}
                >
                  {path}
                </span>
              </span>
            </label>
          ))}
          {!choices.some((choice) => choice.path === destination) && (
            <p className='text-xs break-all text-muted-foreground'>{destination}</p>
          )}
          <Button
            variant='ghost'
            size='sm'
            className='justify-self-start text-xs'
            onClick={() =>
              void (async () => {
                const output = await call(commands.browseBatchOutput)
                if (output) setDestination(output)
              })().catch(() => undefined)
            }
          >
            {t('batch.otherOutput')}
          </Button>
        </fieldset>
        {!count && (
          <p role='status' className='text-xs text-muted-foreground'>
            {t(source.chapters.length ? 'batch.noReadableArchives' : 'batch.noNewArchives')}
          </p>
        )}
        {error && (
          <p role='alert' className='text-xs text-destructive'>
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant='outline' size='sm' disabled={busy} onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            size='sm'
            disabled={busy || !count}
            onClick={() => {
              setBusy(true)
              void onImport(destination).catch((error: unknown) => {
                setBusy(false)
                setError(String(error))
              })
            }}
          >
            {t(busy ? 'batch.adding' : 'batch.addCount', { count })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
