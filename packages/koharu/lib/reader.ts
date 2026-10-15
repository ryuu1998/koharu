'use client'

import { create } from 'zustand'

import {
  commands,
  type ReaderBook,
  type ReaderLibrary,
  type ReaderSession,
  type ReaderSettings,
} from '@koharu/bridge/protocol'

export type ReaderSort = 'name' | 'added' | 'pages' | 'size'
const names = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
const bookName = (book: ReaderBook) => book.name.replace(/\.cbz$/i, '')

export function sortReaderBooks(
  books: readonly ReaderBook[],
  sort: ReaderSort,
  descending: boolean,
) {
  return [...books].sort((a, b) => {
    const difference =
      sort === 'name'
        ? names.compare(bookName(a), bookName(b))
        : sort === 'added'
          ? a.added_at - b.added_at
          : a[sort] - b[sort]
    return (
      (descending ? -difference : difference) ||
      names.compare(bookName(a), bookName(b)) ||
      a.id.localeCompare(b.id)
    )
  })
}

interface ReaderState {
  library: ReaderLibrary | null
  settings: ReaderSettings
  session: ReaderSession | null
  book: ReaderBook | null
  page: number
  offset: number
  busy: boolean
  error: string | null
}

export const useReaderStore = create<ReaderState>(() => ({
  library: null,
  settings: { layout: 'single', direction: 'rtl', fit: 'page', zoom: 1 },
  session: null,
  book: null,
  page: 0,
  offset: 0,
  busy: false,
  error: null,
}))

let sessionGeneration = 0
let libraryGeneration = 0
let settingsGeneration = 0
let settingsQueue: Promise<unknown> = Promise.resolve()
let progressQueue: Promise<unknown> = Promise.resolve()
let sessionQueue: Promise<unknown> = Promise.resolve()

function report(error: unknown) {
  useReaderStore.setState({ error: error instanceof Error ? error.message : String(error) })
}

async function receiveLibrary(request: () => Promise<ReaderLibrary | null>) {
  const generation = ++libraryGeneration
  const settingsVersion = settingsGeneration
  useReaderStore.setState({ busy: true, error: null })
  try {
    const library = await request()
    if (generation !== libraryGeneration) return
    useReaderStore.setState({
      busy: false,
      ...(library
        ? {
            library,
            ...(settingsVersion === settingsGeneration ? { settings: library.settings } : {}),
          }
        : {}),
    })
  } catch (error) {
    if (generation === libraryGeneration) {
      useReaderStore.setState({ busy: false })
      report(error)
    }
  }
}

export const loadReaderLibrary = () => receiveLibrary(commands.getReaderLibrary)
export const addReaderFolder = () => receiveLibrary(commands.browseReaderFolder)
export const refreshReaderLibrary = () => receiveLibrary(commands.refreshReaderLibrary)

export async function openReaderBook(book: ReaderBook) {
  const generation = ++sessionGeneration
  const previous = useReaderStore.getState().session
  const savedProgress = saveReaderPosition()
  useReaderStore.setState({ busy: true, error: null, session: null, book: null })
  const pending = sessionQueue
    .catch(() => undefined)
    .then(async () => {
      await savedProgress
      if (previous) await commands.closeReaderBook(previous.id)
      if (generation !== sessionGeneration) return
      const session = await commands.openReaderBook(book.id)
      if (generation !== sessionGeneration) {
        await commands.closeReaderBook(session.id)
        return
      }
      const currentBook =
        useReaderStore.getState().library?.books.find((item) => item.id === book.id) ?? book
      useReaderStore.setState({
        session,
        book: currentBook,
        page: session.page,
        offset: currentBook.progress?.offset ?? 0,
        busy: false,
      })
    })
  sessionQueue = pending.catch((error: unknown) => {
    if (generation === sessionGeneration) {
      useReaderStore.setState({ busy: false })
      report(error)
    }
  })
  await sessionQueue
}

export async function closeReaderBook() {
  ++sessionGeneration
  const { session } = useReaderStore.getState()
  const savedProgress = saveReaderPosition()
  useReaderStore.setState({ session: null, book: null, busy: false })
  sessionQueue = sessionQueue
    .catch(() => undefined)
    .then(async () => {
      await savedProgress
      if (session) await commands.closeReaderBook(session.id)
    })
    .catch(report)
  await sessionQueue
}

export function setReaderPage(page: number, offset = 0) {
  const { session } = useReaderStore.getState()
  if (!session || !Number.isFinite(page)) return
  useReaderStore.setState({
    page: Math.max(0, Math.min(session.pages.length - 1, Math.floor(page))),
    offset,
  })
}

export function saveReaderPosition(finished?: boolean) {
  const { book, page, offset, library } = useReaderStore.getState()
  if (!book) return progressQueue
  const progress = { page, offset, finished: finished ?? book.progress?.finished ?? false }
  const updated = { ...book, progress }
  useReaderStore.setState({
    book: updated,
    ...(library
      ? {
          library: {
            ...library,
            books: library.books.map((item) => (item.id === book.id ? updated : item)),
          },
        }
      : {}),
  })
  progressQueue = progressQueue
    .catch(() => undefined)
    .then(() => commands.saveReaderProgress(book.id, page, offset, progress.finished))
    .catch(report)
  return progressQueue
}

export function updateReaderSettings(change: Partial<ReaderSettings>) {
  const settings = { ...useReaderStore.getState().settings, ...change }
  if (settings.layout === 'webtoon') settings.fit = 'width'
  settings.zoom = Math.max(0.25, Math.min(4, settings.zoom))
  const generation = ++settingsGeneration
  useReaderStore.setState({ settings, error: null })
  const pending = settingsQueue
    .catch(() => undefined)
    .then(() => commands.saveReaderSettings(settings))
  settingsQueue = pending
    .then((saved) => {
      if (generation === settingsGeneration) useReaderStore.setState({ settings: saved })
    })
    .catch(report)
  return settingsQueue
}
