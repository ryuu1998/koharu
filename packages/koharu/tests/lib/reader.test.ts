import { beforeEach, describe, expect, it, vi } from 'vitest'

import {
  addReaderFolder,
  closeReaderBook,
  loadReaderLibrary,
  openReaderBook,
  refreshReaderLibrary,
  saveReaderPosition,
  setReaderPage,
  sortReaderBooks,
  updateReaderSettings,
  useReaderStore,
} from '@/lib/reader'
import {
  commands,
  type ReaderBook,
  type ReaderLibrary,
  type ReaderSession,
  type ReaderSettings,
} from '@koharu/bridge/protocol'

const settings: ReaderSettings = { layout: 'single', direction: 'rtl', fit: 'page', zoom: 1 }

function book(id: string, change: Partial<ReaderBook> = {}): ReaderBook {
  return {
    id,
    path: `C:/Reader/${id}.cbz`,
    folder: 'C:/Reader',
    name: `${id}.cbz`,
    pages: 3,
    size: 1024,
    added_at: 100,
    thumbnail: null,
    error: null,
    progress: null,
    ...change,
  }
}

const first = book('Chapter 2')
const second = book('Chapter 10')

function library(change: Partial<ReaderLibrary> = {}): ReaderLibrary {
  return {
    folders: [],
    books: [first, second],
    settings,
    folder_errors: [],
    ...change,
  }
}

function session(id: string, page = 0): ReaderSession {
  return {
    id,
    name: id,
    page,
    pages: Array.from({ length: 3 }, (_, index) => ({
      name: `${index + 1}.png`,
      mime: 'image/png',
    })),
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

beforeEach(async () => {
  useReaderStore.setState({
    library: null,
    settings: { ...settings },
    session: null,
    book: null,
    page: 0,
    offset: 0,
    busy: false,
    error: null,
  })
  vi.spyOn(commands, 'closeReaderBook').mockResolvedValue(null)
  vi.spyOn(commands, 'saveReaderProgress').mockResolvedValue(null)
  vi.spyOn(commands, 'saveReaderSettings').mockImplementation(async (value) => value)
  await closeReaderBook()
})

describe('reader library and session ownership', () => {
  it('sorts chapters naturally without mutating library order, with stable numeric-sort ties', () => {
    const chapters = [second, book('Chapter 2.5'), first]
    expect(sortReaderBooks(chapters, 'name', false).map((entry) => entry.name)).toEqual([
      'Chapter 2.cbz',
      'Chapter 2.5.cbz',
      'Chapter 10.cbz',
    ])
    expect(sortReaderBooks(chapters, 'name', true).map((entry) => entry.name)).toEqual([
      'Chapter 10.cbz',
      'Chapter 2.5.cbz',
      'Chapter 2.cbz',
    ])
    expect(sortReaderBooks(chapters, 'pages', true).map((entry) => entry.id)).toEqual([
      'Chapter 2',
      'Chapter 2.5',
      'Chapter 10',
    ])
    expect(chapters[0]).toBe(second)
  })

  it('keeps the persisted folder returned by the native picker when opening the library again', async () => {
    const saved = library({ folders: ['C:/Reader'] })
    vi.spyOn(commands, 'browseReaderFolder').mockResolvedValue(saved)
    const get = vi.spyOn(commands, 'getReaderLibrary').mockResolvedValue(saved)
    await addReaderFolder()
    expect(useReaderStore.getState().library?.folders).toEqual(['C:/Reader'])
    useReaderStore.setState({ library: null })
    await loadReaderLibrary()
    expect(get).toHaveBeenCalledOnce()
    expect(useReaderStore.getState().library?.folders).toEqual(['C:/Reader'])
  })

  it('preserves the existing library when folder selection is cancelled or refresh fails', async () => {
    const existing = library()
    useReaderStore.setState({ library: existing })
    vi.spyOn(commands, 'browseReaderFolder').mockResolvedValue(null)
    await addReaderFolder()
    expect(useReaderStore.getState().library).toBe(existing)
    vi.spyOn(commands, 'refreshReaderLibrary').mockRejectedValue(new Error('Folder is unavailable'))
    await refreshReaderLibrary()
    expect(useReaderStore.getState()).toMatchObject({
      library: existing,
      error: 'Folder is unavailable',
      busy: false,
    })
  })

  it('ignores a stale library response after a newer folder selection', async () => {
    const stale = deferred<ReaderLibrary>()
    vi.spyOn(commands, 'getReaderLibrary').mockReturnValue(stale.promise)
    const oldLoad = loadReaderLibrary()
    const newest = library({ books: [second] })
    vi.spyOn(commands, 'browseReaderFolder').mockResolvedValue(newest)
    await addReaderFolder()
    stale.resolve(library({ books: [first] }))
    await oldLoad
    expect(useReaderStore.getState().library).toBe(newest)
  })

  it('closes a session that arrives after the reader was closed', async () => {
    const pending = deferred<ReaderSession>()
    vi.spyOn(commands, 'openReaderBook').mockReturnValue(pending.promise)
    const opening = openReaderBook(first)
    await vi.waitFor(() => expect(commands.openReaderBook).toHaveBeenCalledOnce())
    const closing = closeReaderBook()
    pending.resolve(session('closed-before-open'))
    await Promise.all([opening, closing])
    expect(commands.closeReaderBook).toHaveBeenCalledWith('closed-before-open')
    expect(useReaderStore.getState()).toMatchObject({ session: null, book: null, busy: false })
  })

  it('keeps the new chapter visible when an earlier open request completes late', async () => {
    const late = deferred<ReaderSession>()
    vi.spyOn(commands, 'openReaderBook')
      .mockReturnValueOnce(late.promise)
      .mockResolvedValueOnce(session('current', 1))
    const oldOpen = openReaderBook(first)
    await vi.waitFor(() => expect(commands.openReaderBook).toHaveBeenCalledOnce())
    const newOpen = openReaderBook(second)
    expect(commands.openReaderBook).toHaveBeenCalledTimes(1)
    late.resolve(session('stale'))
    await Promise.all([oldOpen, newOpen])
    expect(commands.closeReaderBook).toHaveBeenCalledWith('stale')
    expect(useReaderStore.getState()).toMatchObject({
      session: { id: 'current' },
      book: second,
      page: 1,
    })
  })

  it('clamps page navigation to this chapter and saves position before releasing its session', async () => {
    vi.spyOn(commands, 'openReaderBook').mockResolvedValue(session('reading'))
    await openReaderBook(first)
    setReaderPage(-1)
    expect(useReaderStore.getState().page).toBe(0)
    setReaderPage(900, 0.4)
    expect(useReaderStore.getState().page).toBe(2)
    await closeReaderBook()
    await saveReaderPosition()
    expect(commands.saveReaderProgress).toHaveBeenCalledWith(first.id, 2, 0.4, false)
    expect(commands.closeReaderBook).toHaveBeenCalledWith('reading')
  })
})

describe('global reader settings', () => {
  it('retains layout, direction, fit and zoom when moving between chapters', async () => {
    vi.spyOn(commands, 'openReaderBook')
      .mockResolvedValueOnce(session('one'))
      .mockResolvedValueOnce(session('two'))
    await openReaderBook(first)
    await updateReaderSettings({ layout: 'webtoon', direction: 'ltr', fit: 'width', zoom: 1.5 })
    await openReaderBook(second)
    expect(useReaderStore.getState().settings).toEqual({
      layout: 'webtoon',
      direction: 'ltr',
      fit: 'width',
      zoom: 1.5,
    })
  })

  it('serializes saves and prevents an old response from replacing newer settings', async () => {
    const firstSave = deferred<ReaderSettings>()
    const save = vi
      .mocked(commands.saveReaderSettings)
      .mockReturnValueOnce(firstSave.promise)
      .mockImplementationOnce(async (value) => value)
    const earlier = updateReaderSettings({ layout: 'spread' })
    await Promise.resolve()
    await Promise.resolve()
    const newest = updateReaderSettings({ direction: 'ltr' })
    expect(save).toHaveBeenCalledTimes(1)
    expect(useReaderStore.getState().settings).toMatchObject({ layout: 'spread', direction: 'ltr' })
    firstSave.resolve({ ...settings, layout: 'spread' })
    await Promise.all([earlier, newest])
    expect(save).toHaveBeenCalledTimes(2)
    expect(useReaderStore.getState().settings).toMatchObject({ layout: 'spread', direction: 'ltr' })
  })

  it('does not reset recently changed settings when a library request resolves', async () => {
    const pending = deferred<ReaderLibrary>()
    vi.spyOn(commands, 'getReaderLibrary').mockReturnValue(pending.promise)
    const loading = loadReaderLibrary()
    await updateReaderSettings({ fit: 'width' })
    pending.resolve(library())
    await loading
    expect(useReaderStore.getState().settings.fit).toBe('width')
  })

  it('bounds zoom and supports returning to the default scale', async () => {
    await updateReaderSettings({ zoom: 99 })
    expect(useReaderStore.getState().settings.zoom).toBe(4)
    await updateReaderSettings({ zoom: 0 })
    expect(useReaderStore.getState().settings.zoom).toBe(0.25)
    await updateReaderSettings({ zoom: 1 })
    expect(useReaderStore.getState().settings.zoom).toBe(1)
  })
})
