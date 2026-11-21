'use client'

import {
  ArrowDown,
  ArrowUp,
  BookOpen,
  FolderOpen,
  GalleryVerticalEnd,
  LoaderCircle,
  PanelBottom,
  RefreshCw,
  Search,
  SlidersHorizontal,
  TriangleAlert,
} from 'lucide-react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  addReaderFolder,
  closeReaderBook,
  loadReaderLibrary,
  openReaderBook,
  refreshReaderLibrary,
  sortReaderBooks,
  type ReaderSort,
  useReaderStore,
} from '@/lib/reader'
import { Button } from '@koharu/ui/components/button'

import { ReaderView } from './ReaderView'

import './library.css'

export function LibraryPage() {
  const { t } = useTranslation()
  const library = useReaderStore((state) => state.library)
  const session = useReaderStore((state) => state.session)
  const busy = useReaderStore((state) => state.busy)
  const error = useReaderStore((state) => state.error)
  const [search, setSearch] = useState('')
  const [folder, setFolder] = useState('')
  const [filter, setFilter] = useState<'all' | 'reading' | 'unread' | 'finished'>('all')
  const [sort, setSort] = useState<ReaderSort>('name')
  const [descending, setDescending] = useState(false)
  const [caption, setCaption] = useState(true)
  const [cardSize, setCardSize] = useState(96)
  const [foldersOpen, setFoldersOpen] = useState(false)
  const [desktopFoldersOpen, setDesktopFoldersOpen] = useState(true)
  const [narrow, setNarrow] = useState(false)
  const libraryViewport = useRef<HTMLDivElement>(null)
  const libraryScroll = useRef(0)

  useLayoutEffect(() => {
    if (!session && libraryViewport.current)
      libraryViewport.current.scrollTop = libraryScroll.current
  }, [session])

  useEffect(() => {
    const media = window.matchMedia('(max-width:800px)')
    const update = () => setNarrow(media.matches)
    update()
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [])

  useEffect(() => {
    if (!useReaderStore.getState().library) void loadReaderLibrary()
    return () => {
      void closeReaderBook()
    }
  }, [])

  const books = useMemo(
    () =>
      sortReaderBooks(
        (library?.books ?? []).filter(
          (book) =>
            (!folder || book.folder === folder) &&
            book.name.toLocaleLowerCase().includes(search.toLocaleLowerCase()) &&
            (filter === 'all' ||
              (filter === 'finished'
                ? book.progress?.finished
                : filter === 'reading'
                  ? book.progress && !book.progress.finished
                  : !book.progress)),
        ),
        sort,
        descending,
      ),
    [library, folder, search, filter, sort, descending],
  )

  return (
    <main className='reader-library'>
      {session ? (
        <ReaderView key={session.id} session={session} />
      ) : (
        <>
          <header className='reader-library-toolbar'>
            <Button
              variant='ghost'
              size='icon-sm'
              aria-label={t('reader.folders')}
              aria-expanded={narrow ? foldersOpen : desktopFoldersOpen}
              onClick={() =>
                narrow ? setFoldersOpen(!foldersOpen) : setDesktopFoldersOpen(!desktopFoldersOpen)
              }
            >
              <FolderOpen />
            </Button>
            <h1>{t('reader.library')}</h1>
            <label className='reader-search'>
              <Search aria-hidden='true' />
              <input
                type='search'
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t('reader.search')}
                aria-label={t('reader.search')}
              />
            </label>
            <Button
              variant='ghost'
              size='icon-sm'
              disabled={busy}
              aria-label={t('reader.refresh')}
              onClick={() => void refreshReaderLibrary()}
            >
              <RefreshCw className={busy ? 'animate-spin' : ''} />
            </Button>
            <Button
              variant='outline'
              size='sm'
              disabled={busy}
              onClick={() => void addReaderFolder()}
            >
              <FolderOpen />
              {t('reader.addFolder')}
            </Button>
          </header>
          <div className='reader-library-body'>
            <aside
              className={`reader-library-sidebar ${foldersOpen ? 'is-open' : ''} ${desktopFoldersOpen ? '' : 'is-collapsed'}`}
              aria-label={t('reader.folders')}
            >
              {(['all', 'reading', 'unread', 'finished'] as const).map((value) => (
                <button
                  key={value}
                  type='button'
                  aria-pressed={!folder && filter === value}
                  onClick={() => {
                    setFolder('')
                    setFilter(value)
                    setFoldersOpen(false)
                  }}
                >
                  {t(`reader.filter.${value}`)}
                </button>
              ))}
              {!!library?.folders.length && <h2>{t('reader.folders')}</h2>}
              {library?.folders.map((path) => (
                <button
                  key={path}
                  type='button'
                  title={path}
                  aria-pressed={folder === path}
                  onClick={() => {
                    setFolder(path)
                    setFilter('all')
                    setFoldersOpen(false)
                  }}
                >
                  <FolderOpen aria-hidden='true' />
                  <span>{path.split(/[\\/]/).filter(Boolean).at(-1)}</span>
                </button>
              ))}
            </aside>
            <div
              ref={libraryViewport}
              className='reader-library-content'
              onScroll={(event) => {
                libraryScroll.current = event.currentTarget.scrollTop
              }}
            >
              <div className='reader-library-controls'>
                <label className='reader-sort'>
                  <SlidersHorizontal aria-hidden='true' />
                  <select
                    aria-label={t('reader.sortBy')}
                    value={sort}
                    onChange={(event) => {
                      setSort(event.target.value as ReaderSort)
                      setDescending(false)
                    }}
                  >
                    {(['name', 'added', 'pages', 'size'] as const).map((value) => (
                      <option key={value} value={value}>
                        {t(`reader.sort.${value}`)}
                      </option>
                    ))}
                  </select>
                </label>
                <Button
                  variant='ghost'
                  size='icon-sm'
                  aria-label={t(descending ? 'reader.descending' : 'reader.ascending')}
                  onClick={() => setDescending(!descending)}
                >
                  {descending ? <ArrowDown /> : <ArrowUp />}
                </Button>
                <span className='reader-library-count'>
                  {t('reader.bookCount', { count: books.length })}
                </span>
                <Button
                  variant='ghost'
                  size='icon-sm'
                  aria-label={t(caption ? 'reader.overlayCards' : 'reader.captionCards')}
                  onClick={() => setCaption(!caption)}
                >
                  {caption ? <GalleryVerticalEnd /> : <PanelBottom />}
                </Button>
                <input
                  className='reader-card-slider'
                  type='range'
                  min='64'
                  max='160'
                  step='8'
                  value={cardSize}
                  aria-label={t('reader.cardSize')}
                  onChange={(event) => setCardSize(Number(event.target.value))}
                />
              </div>
              {error && (
                <p className='reader-error' role='alert'>
                  {error}
                </p>
              )}
              {library?.folder_errors?.map((failure) => (
                <p className='reader-error' key={failure.path}>
                  <TriangleAlert aria-hidden='true' />
                  <span>
                    {failure.path}: {failure.error}
                  </span>
                </p>
              ))}
              {busy && !library ? (
                <div className='reader-empty'>
                  <LoaderCircle className='animate-spin' />
                  <p>{t('reader.loading')}</p>
                </div>
              ) : !books.length ? (
                <div className='reader-empty'>
                  <BookOpen />
                  <h2>{t(library?.books.length ? 'reader.noMatches' : 'reader.empty')}</h2>
                  <p>{t('reader.emptyHint')}</p>
                  {!library?.folders.length && (
                    <Button variant='outline' onClick={() => void addReaderFolder()}>
                      {t('reader.addFolder')}
                    </Button>
                  )}
                </div>
              ) : (
                <div
                  className='reader-book-grid'
                  style={{ gridTemplateColumns: `repeat(auto-fill, ${cardSize + 16}px)` }}
                >
                  {books.map((book) => (
                    <button
                      key={book.id}
                      type='button'
                      className={`reader-book ${caption ? 'is-caption' : 'is-overlay'}`}
                      disabled={busy || !!book.error}
                      title={book.error ?? book.name}
                      onClick={() => void openReaderBook(book)}
                    >
                      <span
                        className='reader-book-cover'
                        style={{ width: cardSize, height: (cardSize * 4) / 3 }}
                      >
                        {book.thumbnail ? (
                          <img src={book.thumbnail} alt='' loading='lazy' />
                        ) : (
                          <BookOpen aria-hidden='true' />
                        )}
                        {!caption && (
                          <span className='reader-book-overlay-title'>
                            {book.name.replace(/\.cbz$/i, '')}
                          </span>
                        )}
                        {book.progress && !book.progress.finished && (
                          <span
                            className='reader-book-progress'
                            style={{
                              width: `${((book.progress.page + 1) / Math.max(1, book.pages)) * 100}%`,
                            }}
                          />
                        )}
                      </span>
                      {caption && (
                        <span className='reader-book-title'>
                          {book.name.replace(/\.cbz$/i, '')}
                        </span>
                      )}
                      <span className='reader-book-meta'>
                        {book.error
                          ? t('reader.unavailable')
                          : book.progress?.finished
                            ? t('reader.finished')
                            : t('reader.pages', { count: book.pages })}
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>
        </>
      )}
    </main>
  )
}
