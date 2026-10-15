'use client'

import { ArrowDown, ArrowUp, ChevronDown, GalleryVerticalEnd, PanelBottom } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { type BatchSort, sortBatchChapters, useBatchStore } from '@/lib/batch'
import { Button } from '@koharu/ui/components/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'
import { Slider } from '@koharu/ui/components/slider'

export function BatchViewControls({ busy }: { busy: boolean }) {
  const { t } = useTranslation()
  const cardSize = useBatchStore((s) => s.cardSize)
  const cardLayout = useBatchStore((s) => s.cardLayout)
  const toggleCardLayout = useBatchStore((s) => s.toggleCardLayout)
  const sortBy = useBatchStore((s) => s.sortBy)
  const descending = useBatchStore((s) => s.sortDescending)
  const setCardSize = useBatchStore((s) => s.setCardSize)
  const setSort = useBatchStore((s) => s.setSort)
  const source = useBatchStore((s) => s.source)
  const selected = useBatchStore((s) => s.selected)
  const exported = useBatchStore((s) => s.exported)
  const statuses = useBatchStore((s) => s.statuses)
  const running = useBatchStore((s) => s.running)
  const select = useBatchStore((s) => s.select)
  const pending = source?.chapters.filter((c) => !c.error && !exported.includes(c.path)) ?? []
  const allSelected = pending.length > 0 && pending.every((c) => selected.includes(c.path))
  const Direction = descending ? ArrowDown : ArrowUp
  const direction = t(`batch.sort.${sortBy}.${descending ? 'descending' : 'ascending'}`)

  return (
    <>
      <Button
        variant='ghost'
        size='icon-sm'
        onClick={toggleCardLayout}
        aria-label={t(cardLayout === 'overlay' ? 'batch.layout.caption' : 'batch.layout.overlay')}
        title={t(cardLayout === 'overlay' ? 'batch.layout.caption' : 'batch.layout.overlay')}
      >
        {cardLayout === 'overlay' ? (
          <PanelBottom className='size-4' strokeWidth={1.75} />
        ) : (
          <GalleryVerticalEnd className='size-4' strokeWidth={1.75} />
        )}
      </Button>
      <div
        className='flex shrink-0 items-center gap-2'
        role='group'
        aria-label={t('batch.cardSize')}
      >
        <span className='text-[11px] text-muted-foreground'>{t('batch.cards')}</span>
        <Slider
          aria-label={t('batch.cardSize')}
          min={64}
          max={160}
          step={8}
          value={cardSize}
          aria-valuetext={`${Math.round((cardSize / 96) * 100)}%`}
          onValueChange={setCardSize}
          className='shrink-0 [&_[data-slot=slider-thumb]]:size-2.5'
          style={{ width: 80 }}
        />
        <span className='w-8 shrink-0 text-right text-[10px] text-muted-foreground tabular-nums'>
          {Math.round((cardSize / 96) * 100)}%
        </span>
      </div>
      <Button
        variant='ghost'
        size='sm'
        className='text-[11px]'
        disabled={busy || !pending.length || (running && allSelected)}
        onClick={() =>
          void select(
            allSelected
              ? []
              : [
                  ...selected,
                  ...sortBatchChapters(pending, sortBy, descending, {
                    statuses,
                    exported,
                    selected,
                  })
                    .filter((c) => !selected.includes(c.path))
                    .map((c) => c.path),
                ],
          ).catch(() => undefined)
        }
      >
        {t(running ? 'batch.addRemaining' : allSelected ? 'batch.deselectAll' : 'batch.selectAll')}
      </Button>
      <div className='flex shrink-0 items-center' role='group' aria-label={t('batch.sort.label')}>
        <Button
          variant='ghost'
          size='sm'
          className='gap-1.5 rounded-e-none pe-1.5 text-[11px]'
          aria-label={t('batch.sort.reverse', { direction })}
          title={direction}
          onClick={() => setSort(sortBy)}
        >
          {t(`batch.sort.${sortBy}.label`)}
          <Direction className='size-3.5' strokeWidth={1.75} aria-hidden='true' />
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant='ghost'
                size='icon-sm'
                className='w-6 rounded-s-none'
                aria-label={t('batch.sort.choose')}
              />
            }
          >
            <ChevronDown className='size-3.5' strokeWidth={1.75} />
          </DropdownMenuTrigger>
          <DropdownMenuContent align='end' className='w-48'>
            {(['added', 'size', 'pages', 'status'] as const).map((field: BatchSort) => (
              <DropdownMenuItem
                key={field}
                className='justify-between text-xs'
                onClick={() => setSort(field)}
                title={field === 'added' ? t('batch.sort.addedHint') : undefined}
              >
                {t(`batch.sort.${field}.label`)}
                {field === sortBy && <Direction className='size-3.5' aria-label={direction} />}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </>
  )
}
