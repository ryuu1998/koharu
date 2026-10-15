'use client'

import { Dialog } from '@base-ui/react/dialog'
import { Minus, Plus, RotateCcw, X } from 'lucide-react'
import type { RefObject } from 'react'
import { useTranslation } from 'react-i18next'

import { updateReaderSettings, useReaderStore } from '@/lib/reader'
import type { ReaderSettings } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'

export function ReaderSettingsPanel({
  open,
  onOpenChange,
  container,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  container: RefObject<HTMLDivElement | null>
}) {
  const { t } = useTranslation()
  const settings = useReaderStore((state) => state.settings)
  const choices = <K extends 'layout' | 'direction' | 'fit'>(
    key: K,
    options: ReaderSettings[K][],
  ) => (
    <div className='reader-option-group'>
      {options.map((value) => (
        <button
          key={value}
          type='button'
          aria-pressed={settings[key] === value}
          onClick={() => void updateReaderSettings({ [key]: value })}
        >
          {t(`reader.${key}.${value}`)}
        </button>
      ))}
    </div>
  )
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange} modal={false}>
      <Dialog.Portal container={container}>
        <Dialog.Popup
          className='reader-settings-sheet'
          finalFocus={() => document.querySelector<HTMLElement>('[data-reader-settings-trigger]')}
        >
          <div className='reader-settings-header'>
            <Dialog.Title>{t('reader.settings')}</Dialog.Title>
            <Dialog.Close
              render={
                <Button variant='ghost' size='icon-sm' aria-label={t('reader.closeSettings')} />
              }
            >
              <X />
            </Dialog.Close>
          </div>
          <div className='reader-settings-body'>
            <section>
              <h3>{t('reader.pageLayout')}</h3>
              {choices('layout', ['single', 'spread', 'webtoon'])}
            </section>
            <section>
              <h3>{t('reader.readingDirection')}</h3>
              {choices('direction', ['rtl', 'ltr'])}
            </section>
            <section>
              <h3>{t('reader.pageFit')}</h3>
              {choices(
                'fit',
                settings.layout === 'webtoon' ? ['width'] : ['page', 'width', 'height', 'original'],
              )}
              {settings.layout === 'webtoon' && (
                <p className='reader-setting-note'>{t('reader.webtoonFit')}</p>
              )}
            </section>
            <section>
              <h3>{t('reader.zoom')}</h3>
              <div className='reader-zoom-row'>
                <Button
                  variant='outline'
                  size='icon-sm'
                  aria-label={t('reader.zoomOut')}
                  onClick={() => void updateReaderSettings({ zoom: settings.zoom - 0.1 })}
                >
                  <Minus />
                </Button>
                <output>{Math.round(settings.zoom * 100)}%</output>
                <Button
                  variant='outline'
                  size='icon-sm'
                  aria-label={t('reader.zoomIn')}
                  onClick={() => void updateReaderSettings({ zoom: settings.zoom + 0.1 })}
                >
                  <Plus />
                </Button>
                <Button
                  variant='ghost'
                  size='icon-sm'
                  aria-label={t('reader.resetZoom')}
                  onClick={() => void updateReaderSettings({ zoom: 1 })}
                >
                  <RotateCcw />
                </Button>
              </div>
            </section>
            <p className='reader-setting-note'>{t('reader.globalSettings')}</p>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
