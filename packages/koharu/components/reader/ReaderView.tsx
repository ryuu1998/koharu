'use client'

import {
  ArrowLeft,
  Check,
  ChevronLeft,
  ChevronRight,
  Maximize,
  Minus,
  Plus,
  RefreshCw,
  RotateCcw,
  SlidersHorizontal,
} from 'lucide-react'
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  closeReaderBook,
  openReaderBook,
  saveReaderPosition,
  setReaderPage,
  updateReaderSettings,
  useReaderStore,
} from '@/lib/reader'
import type { ReaderSession } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'

import { ReaderSettingsPanel } from './ReaderSettingsPanel'
import { useReaderImages } from './useReaderImages'

import './reader.css'

export function ReaderView({ session }: { session: ReaderSession }) {
  const { t } = useTranslation()
  const settings = useReaderStore((state) => state.settings)
  const page = useReaderStore((state) => state.page)
  const book = useReaderStore((state) => state.book)
  const error = useReaderStore((state) => state.error)
  const viewport = useRef<HTMLDivElement>(null)
  const root = useRef<HTMLDivElement>(null)
  const [panel, setPanel] = useState(false)
  const [chrome, setChrome] = useState(true)
  const [size, setSize] = useState({ width: 800, height: 600 })
  const [dimensions, setDimensions] = useState<Record<number, { width: number; height: number }>>(
    {},
  )
  const [gestureZoom, setGestureZoom] = useState<number | null>(null)
  const pendingAnchor = useRef<{ page: number; top: number } | null>(null)
  const webtoon = settings.layout === 'webtoon'
  const zoom = gestureZoom ?? settings.zoom
  const pages =
    settings.layout === 'spread' && page > 0 && page + 1 < session.pages.length
      ? [page, page + 1]
      : [page]
  const { images, errors, recordDimensions } = useReaderImages(session, page, pages.length)

  const navigate = useCallback(
    (index: number) => {
      const next = Math.max(0, Math.min(session.pages.length - 1, index))
      setReaderPage(next)
      if (webtoon)
        viewport.current
          ?.querySelector<HTMLElement>(`[data-reader-page="${next}"]`)
          ?.scrollIntoView({ block: 'start' })
      else if (viewport.current) {
        viewport.current.scrollTop = 0
        viewport.current.scrollLeft = 0
      }
      void saveReaderPosition()
    },
    [session.pages.length, webtoon],
  )

  const turn = useCallback(
    (forward: boolean) => {
      const step = settings.layout === 'spread' ? (forward && page === 0 ? 1 : 2) : 1
      navigate(page + (forward ? step : -step))
    },
    [navigate, page, settings.layout],
  )

  const fullscreen = useCallback(() => {
    const action = document.fullscreenElement
      ? document.exitFullscreen()
      : root.current?.requestFullscreen()
    void action?.catch(() => useReaderStore.setState({ error: t('reader.fullscreenUnavailable') }))
  }, [t])

  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const observer = new ResizeObserver(() =>
      setSize({ width: element.clientWidth, height: element.clientHeight }),
    )
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  useLayoutEffect(() => {
    const anchor = pendingAnchor.current
    const container = viewport.current
    if (!webtoon || !container) return
    const position = useReaderStore.getState()
    const element = container.querySelector<HTMLElement>(
      `[data-reader-page="${anchor?.page ?? position.page}"]`,
    )
    if (element)
      container.scrollTop = anchor
        ? element.offsetTop - anchor.top
        : element.offsetTop + element.offsetHeight * position.offset
    pendingAnchor.current = null
  }, [dimensions, webtoon, zoom, size.width, session.id])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        panel ||
        event.ctrlKey ||
        event.altKey ||
        event.metaKey ||
        (event.target instanceof HTMLElement &&
          event.target.closest(
            'input, select, textarea, button, a, [contenteditable], [role="button"], [role="dialog"]',
          ))
      )
        return
      const element = viewport.current
      const key = event.key
      if (key === 'Escape') {
        setChrome(true)
        return
      }
      if (key.toLowerCase() === 'f') {
        event.preventDefault()
        fullscreen()
        return
      }
      if (['+', '=', '-', '0'].includes(key)) {
        event.preventDefault()
        void updateReaderSettings({
          zoom: key === '0' ? 1 : settings.zoom + (key === '-' ? -0.1 : 0.1),
        })
        return
      }
      if (key === 'Home' || key === 'End') {
        event.preventDefault()
        navigate(key === 'Home' ? 0 : session.pages.length - 1)
        return
      }
      if (['PageDown', 'PageUp', ' '].includes(key)) {
        event.preventDefault()
        const forward = key === 'PageDown' || (key === ' ' && !event.shiftKey)
        if (webtoon)
          element?.scrollBy({ top: (forward ? 1 : -1) * size.height * 0.85, behavior: 'instant' })
        else turn(forward)
        return
      }
      if (key.startsWith('Arrow')) {
        event.preventDefault()
        if (settings.zoom > 1 || webtoon || key === 'ArrowUp' || key === 'ArrowDown') {
          element?.scrollBy({
            left: key === 'ArrowLeft' ? -100 : key === 'ArrowRight' ? 100 : 0,
            top: key === 'ArrowUp' ? -100 : key === 'ArrowDown' ? 100 : 0,
          })
        } else turn((key === 'ArrowRight') === (settings.direction === 'ltr'))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [
    fullscreen,
    navigate,
    panel,
    session.pages.length,
    settings.direction,
    settings.zoom,
    size.height,
    turn,
    webtoon,
  ])

  useEffect(() => {
    const save = () => {
      void saveReaderPosition()
    }
    window.addEventListener('pagehide', save)
    return () => window.removeEventListener('pagehide', save)
  }, [])

  const pointers = useRef(new Map<number, { x: number; y: number }>())
  const gesture = useRef({ x: 0, y: 0, left: 0, top: 0, distance: 0, zoom: 1, pinched: false })
  const currentZoom = useRef(zoom)
  currentZoom.current = zoom
  const pointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY })
    if (pointers.current.size === 1)
      gesture.current = {
        x: event.clientX,
        y: event.clientY,
        left: event.currentTarget.scrollLeft,
        top: event.currentTarget.scrollTop,
        distance: 0,
        zoom,
        pinched: false,
      }
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()]
      gesture.current.distance = Math.hypot(a.x - b.x, a.y - b.y)
      gesture.current.zoom = zoom
      gesture.current.pinched = true
    }
    event.currentTarget.setPointerCapture(event.pointerId)
  }
  const pointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!pointers.current.has(event.pointerId)) return
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY })
    if (pointers.current.size === 2 && gesture.current.distance) {
      const [a, b] = [...pointers.current.values()]
      setGestureZoom(
        Math.max(
          0.25,
          Math.min(
            4,
            (gesture.current.zoom * Math.hypot(a.x - b.x, a.y - b.y)) / gesture.current.distance,
          ),
        ),
      )
    } else if (!gesture.current.pinched && (!webtoon || zoom > 1)) {
      event.currentTarget.scrollLeft = gesture.current.left - event.clientX + gesture.current.x
      event.currentTarget.scrollTop = gesture.current.top - event.clientY + gesture.current.y
    }
  }
  const pointerUp = (event: PointerEvent<HTMLDivElement>, cancelled = false) => {
    if (!pointers.current.has(event.pointerId)) return
    pointers.current.delete(event.pointerId)
    if (gesture.current.pinched) {
      if (!pointers.current.size) {
        void updateReaderSettings({ zoom: currentZoom.current })
        setGestureZoom(null)
      }
      return
    }
    const dx = event.clientX - gesture.current.x
    const dy = event.clientY - gesture.current.y
    if (cancelled) return
    if (
      !webtoon &&
      zoom <= 1 &&
      event.currentTarget.scrollWidth <= event.currentTarget.clientWidth + 1 &&
      Math.abs(dx) > 50 &&
      Math.abs(dx) > Math.abs(dy) * 1.5
    )
      turn(dx < 0 === (settings.direction === 'ltr'))
    else if (Math.abs(dx) < 8 && Math.abs(dy) < 8) {
      const bounds = event.currentTarget.getBoundingClientRect()
      const position = (event.clientX - bounds.left) / bounds.width
      if (!webtoon && zoom <= 1 && (position < 0.25 || position > 0.75))
        turn(position > 0.75 === (settings.direction === 'ltr'))
      else setChrome((visible) => !visible)
    }
  }

  const onScroll = () => {
    if (!webtoon || !viewport.current) return
    const container = viewport.current
    const marker = container.scrollTop + 4
    const elements = container.querySelectorAll<HTMLElement>('[data-reader-page]')
    const current = [...elements].find(
      (element) => element.offsetTop + element.offsetHeight > marker,
    )
    if (current)
      setReaderPage(
        Number(current.dataset.readerPage),
        Math.max(0, Math.min(1, (container.scrollTop - current.offsetTop) / current.offsetHeight)),
      )
  }

  const image = (index: number, continuous = false) => {
    const natural = dimensions[index] ?? { width: 750, height: 1000 }
    const availableWidth = continuous ? size.width : (size.width - 32) / pages.length
    const availableHeight = size.height - 96
    const scale = continuous
      ? availableWidth / natural.width
      : settings.fit === 'original'
        ? 1
        : settings.fit === 'width'
          ? availableWidth / natural.width
          : settings.fit === 'height'
            ? availableHeight / natural.height
            : Math.min(availableWidth / natural.width, availableHeight / natural.height)
    const width = Math.max(1, natural.width * scale * zoom)
    return (
      <div
        key={index}
        data-reader-page={index}
        className={continuous ? 'reader-webtoon-page' : 'reader-paged-image'}
        style={{ width, height: (natural.height / natural.width) * width }}
      >
        {images[index] ? (
          <img
            draggable={false}
            src={images[index]}
            alt={t('reader.page', { page: index + 1 })}
            onLoad={(event) => {
              const img = event.currentTarget
              recordDimensions(index, img.naturalWidth, img.naturalHeight)
              const container = viewport.current
              const current = useReaderStore.getState().page
              const anchor = container?.querySelector<HTMLElement>(
                `[data-reader-page="${current}"]`,
              )
              if (continuous && anchor && container)
                pendingAnchor.current = {
                  page: current,
                  top: anchor.offsetTop - container.scrollTop,
                }
              setDimensions((previous) =>
                previous[index]?.width === img.naturalWidth &&
                previous[index]?.height === img.naturalHeight
                  ? previous
                  : {
                      ...previous,
                      [index]: { width: img.naturalWidth, height: img.naturalHeight },
                    },
              )
            }}
          />
        ) : (
          <span className={errors[index] ? 'reader-error' : 'reader-page-loading'}>
            {errors[index] ?? t('reader.page', { page: index + 1 })}
          </span>
        )}
      </div>
    )
  }

  return (
    <div ref={root} className='reader-view'>
      <div
        ref={viewport}
        className={`reader-viewport ${webtoon ? 'is-webtoon' : 'is-paged'} ${zoom > 1 ? 'is-zoomed' : ''}`}
        onScroll={onScroll}
        onScrollEnd={() => {
          void saveReaderPosition()
        }}
        onPointerDown={pointerDown}
        onPointerMove={pointerMove}
        onPointerUp={(event) => pointerUp(event)}
        onPointerCancel={(event) => pointerUp(event, true)}
        onDoubleClick={(event) => {
          const bounds = event.currentTarget.getBoundingClientRect()
          const position = (event.clientX - bounds.left) / bounds.width
          if (position >= 0.25 && position <= 0.75)
            void updateReaderSettings({ zoom: settings.zoom > 1 ? 1 : 2 })
        }}
      >
        {webtoon ? (
          <div className='reader-webtoon-strip'>
            {session.pages.map((_, index) => image(index, true))}
          </div>
        ) : (
          <div className={`reader-page-spread ${settings.direction === 'rtl' ? 'is-rtl' : ''}`}>
            {pages.map((index) => image(index))}
          </div>
        )}
      </div>
      <div className={`reader-chrome ${chrome ? '' : 'is-hidden'}`} inert={!chrome}>
        <header className='reader-topbar'>
          <Button variant='ghost' size='sm' onClick={() => void closeReaderBook()}>
            <ArrowLeft />
            {t('reader.library')}
          </Button>
          <h1 title={session.name}>{session.name.replace(/\.cbz$/i, '')}</h1>
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={t('reader.fullscreen')}
            onClick={fullscreen}
          >
            <Maximize />
          </Button>
          <Button
            variant='ghost'
            size='sm'
            aria-label={t('reader.settings')}
            data-reader-settings-trigger
            onClick={() => setPanel(true)}
          >
            <SlidersHorizontal />
            <span className='reader-settings-label'>{t('reader.settings')}</span>
          </Button>
        </header>
        <footer className='reader-bottombar'>
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={t('reader.previous')}
            disabled={page === 0}
            onClick={() => turn(false)}
          >
            {settings.direction === 'rtl' ? <ChevronRight /> : <ChevronLeft />}
          </Button>
          <input
            type='range'
            min='0'
            max={session.pages.length - 1}
            value={page}
            aria-label={t('reader.goToPage')}
            onChange={(event) => navigate(Number(event.target.value))}
          />
          <label className='reader-page-number'>
            <input
              type='number'
              min='1'
              max={session.pages.length}
              value={page + 1}
              aria-label={t('reader.pageNumber')}
              onChange={(event) => {
                if (event.target.value) navigate(Number(event.target.value) - 1)
              }}
            />
            <span>/ {session.pages.length}</span>
          </label>
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={t('reader.next')}
            disabled={page + pages.length >= session.pages.length}
            onClick={() => turn(true)}
          >
            {settings.direction === 'rtl' ? <ChevronLeft /> : <ChevronRight />}
          </Button>
          <span className='reader-bottom-separator' />
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={t('reader.zoomOut')}
            onClick={() => void updateReaderSettings({ zoom: settings.zoom - 0.1 })}
          >
            <Minus />
          </Button>
          <output className='reader-zoom-label'>{Math.round(zoom * 100)}%</output>
          <Button
            variant='ghost'
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
          {page + pages.length >= session.pages.length && (
            <Button
              variant='ghost'
              size='sm'
              onClick={() => void saveReaderPosition(!book?.progress?.finished)}
            >
              <Check />
              {t(book?.progress?.finished ? 'reader.markUnread' : 'reader.markFinished')}
            </Button>
          )}
        </footer>
      </div>
      {(error || errors[page]) && (
        <div className='reader-view-error' role='alert'>
          <p>{error ?? errors[page]}</p>
          {book && (
            <Button variant='outline' size='sm' onClick={() => void openReaderBook(book)}>
              <RefreshCw />
              {t('reader.reload')}
            </Button>
          )}
        </div>
      )}
      {!chrome && (
        <button
          type='button'
          className='reader-show-controls'
          aria-label={t('reader.showControls')}
          onClick={() => setChrome(true)}
        >
          <SlidersHorizontal />
        </button>
      )}
      <ReaderSettingsPanel open={panel} onOpenChange={setPanel} container={root} />
    </div>
  )
}
