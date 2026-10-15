import { act, fireEvent, render, renderHook, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { LibraryPage } from '@/components/reader/LibraryPage'
import { ReaderView } from '@/components/reader/ReaderView'
import { useReaderImages } from '@/components/reader/useReaderImages'
import { useReaderStore } from '@/lib/reader'
import { commands, type ReaderBook, type ReaderSession } from '@koharu/bridge/protocol'

const book: ReaderBook = {
  id: 'chapter',
  path: 'C:/Reader/chapter.cbz',
  folder: 'C:/Reader',
  name: 'chapter.cbz',
  pages: 3,
  size: 1024,
  added_at: 100,
  thumbnail: null,
  error: null,
  progress: null,
}

function session(id: string, pages = 1): ReaderSession {
  return {
    id,
    name: `${id}.cbz`,
    page: 0,
    pages: Array.from({ length: pages }, (_, index) => ({
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

beforeEach(() => {
  HTMLElement.prototype.scrollBy = vi.fn()
  useReaderStore.setState({
    library: null,
    book: null,
    session: null,
    page: 0,
    offset: 0,
    busy: false,
    error: null,
    settings: { layout: 'single', direction: 'rtl', fit: 'page', zoom: 1 },
  })
  vi.spyOn(commands, 'saveReaderSettings').mockImplementation(async (settings) => settings)
  vi.spyOn(commands, 'saveReaderProgress').mockResolvedValue(null)
  vi.spyOn(commands, 'closeReaderBook').mockResolvedValue(null)
  vi.spyOn(commands, 'getReaderPage').mockResolvedValue([137, 80, 78, 71])
  let urls = 0
  vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:reader-${++urls}`)
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
})

describe('reader image lifetime', () => {
  it('loads the new chapter without displaying a late response from the old chapter', async () => {
    const previous = deferred<number[]>()
    const current = deferred<number[]>()
    vi.mocked(commands.getReaderPage).mockImplementation((id) =>
      id === 'old' ? previous.promise : current.promise,
    )
    const { result, rerender } = renderHook(({ active }) => useReaderImages(active, 0), {
      initialProps: { active: session('old') },
    })
    rerender({ active: session('new') })
    await waitFor(() => expect(commands.getReaderPage).toHaveBeenCalledWith('new', 0))
    await act(async () => {
      current.resolve([2])
      await current.promise
    })
    await waitFor(() => expect(result.current.images[0]).toBe('blob:reader-1'))
    await act(async () => {
      previous.resolve([1])
      await previous.promise
    })
    expect(result.current.images[0]).toBe('blob:reader-1')
    expect(URL.createObjectURL).toHaveBeenCalledOnce()
  })

  it('revokes page URLs on close and discards bytes received after close', async () => {
    const late = deferred<number[]>()
    vi.mocked(commands.getReaderPage).mockImplementation((_id, index) =>
      index === 0 ? Promise.resolve([1]) : late.promise,
    )
    const active = session('reading', 2)
    const { result, unmount } = renderHook(() => useReaderImages(active, 0))
    await waitFor(() => expect(result.current.images[0]).toBe('blob:reader-1'))
    act(() => result.current.recordDimensions(0, 800, 1200))
    await waitFor(() => expect(commands.getReaderPage).toHaveBeenCalledWith('reading', 1))
    unmount()
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:reader-1')
    await act(async () => {
      late.resolve([2])
      await late.promise
    })
    expect(URL.createObjectURL).toHaveBeenCalledOnce()
  })

  it('bounds prefetch around the visible page instead of decoding the whole chapter', async () => {
    const active = session('long-chapter', 80)
    const { result, rerender } = renderHook(({ page }) => useReaderImages(active, page), {
      initialProps: { page: 0 },
    })
    await waitFor(() => expect(Object.keys(result.current.images)).toEqual(['0']))
    expect(vi.mocked(commands.getReaderPage).mock.calls.map((call) => call[1])).toEqual([0])
    act(() => result.current.recordDimensions(0, 800, 1200))
    await waitFor(() => expect(Object.keys(result.current.images)).toEqual(['0', '1']))
    rerender({ page: 10 })
    await waitFor(() => expect(Object.keys(result.current.images)).toEqual(['10']))
    act(() => result.current.recordDimensions(10, 8000, 8000))
    expect(vi.mocked(commands.getReaderPage).mock.calls.map((call) => call[1])).toEqual([0, 1, 10])
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2)
  })
})

describe('reader controls', () => {
  function showReader() {
    const active = session('chapter', 3)
    useReaderStore.setState({ session: active, book })
    return render(<ReaderView session={active} />)
  }

  it('opens layout choices in a settings overlay and retains the choice after closing it', async () => {
    showReader()
    expect(screen.queryByRole('button', { name: 'Single page' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Reading settings' }))
    const panel = await screen.findByRole('dialog', { name: 'Reading settings' })
    expect(within(panel).getByRole('button', { name: 'Single page' })).toBeVisible()
    expect(within(panel).getByRole('button', { name: 'Two pages' })).toBeVisible()
    fireEvent.click(within(panel).getByRole('button', { name: 'Webtoon' }))
    await waitFor(() => expect(useReaderStore.getState().settings.layout).toBe('webtoon'))
    fireEvent.keyDown(panel, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(useReaderStore.getState().settings.layout).toBe('webtoon')
  })

  it('uses RTL spatial arrows but leaves typing and focused controls alone', async () => {
    showReader()
    fireEvent.keyDown(window, { key: 'ArrowLeft' })
    expect(useReaderStore.getState().page).toBe(1)
    fireEvent.keyDown(window, { key: 'ArrowRight' })
    expect(useReaderStore.getState().page).toBe(0)
    fireEvent.keyDown(screen.getByRole('spinbutton', { name: 'Page number' }), { key: 'ArrowLeft' })
    expect(useReaderStore.getState().page).toBe(0)
    fireEvent.keyDown(screen.getByRole('button', { name: 'Next page' }), { key: ' ' })
    expect(useReaderStore.getState().page).toBe(0)
  })

  it('adds a folder through the visible Library action and shows its chapters', async () => {
    const settings = useReaderStore.getState().settings
    useReaderStore.setState({ library: { folders: [], books: [], settings, folder_errors: [] } })
    const picker = vi.spyOn(commands, 'browseReaderFolder').mockResolvedValue({
      folders: ['C:/Reader'],
      books: [book],
      settings,
      folder_errors: [],
    })
    render(<LibraryPage />)
    fireEvent.click(screen.getAllByRole('button', { name: 'Add folder' })[0])
    await waitFor(() => expect(picker).toHaveBeenCalledOnce())
    expect(await screen.findByRole('button', { name: /chapter.*3 pages/i })).toBeEnabled()
    expect(useReaderStore.getState().library?.folders).toEqual(['C:/Reader'])
  })

  it('keeps the visible Webtoon page and fractional offset anchored when zoom changes', async () => {
    vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (
      this: HTMLElement,
    ) {
      return this.dataset.readerPage === undefined ? 0 : Number.parseFloat(this.style.height)
    })
    vi.spyOn(HTMLElement.prototype, 'offsetTop', 'get').mockImplementation(function (
      this: HTMLElement,
    ) {
      let top = 0
      let previous = this.previousElementSibling as HTMLElement | null
      while (previous) {
        top += previous.offsetHeight
        previous = previous.previousElementSibling as HTMLElement | null
      }
      return top
    })
    useReaderStore.setState({
      page: 1,
      offset: 0.25,
      settings: { layout: 'webtoon', direction: 'rtl', fit: 'width', zoom: 1 },
    })
    const { container } = showReader()
    const visible = container.querySelector<HTMLElement>('[data-reader-page="1"]')!
    const viewport = visible.parentElement!.parentElement!
    const expected = () => visible.offsetTop + visible.offsetHeight * 0.25
    await waitFor(() => expect(viewport.scrollTop).toBeCloseTo(expected()))
    const previousTop = viewport.scrollTop
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
    await waitFor(() => expect(useReaderStore.getState().settings.zoom).toBeGreaterThan(1))
    await waitFor(() => expect(viewport.scrollTop).toBeCloseTo(expected()))
    expect(viewport.scrollTop).toBeGreaterThan(previousTop)
    expect(useReaderStore.getState()).toMatchObject({ page: 1, offset: 0.25 })
  })

  it('returns to the same library scroll position after reading a chapter', async () => {
    const { LibraryPage } = await import('@/components/reader/LibraryPage')
    useReaderStore.setState({
      library: {
        folders: [book.folder],
        books: [book],
        folder_errors: [],
        settings: useReaderStore.getState().settings,
      },
    })
    const { container } = render(<LibraryPage />)
    const library = container.querySelector<HTMLElement>('.reader-library-content')!
    library.scrollTop = 240
    fireEvent.scroll(library)
    act(() => useReaderStore.setState({ session: session('reading', 3), book }))
    expect(container.querySelector('.reader-library-content')).toBeNull()
    act(() => useReaderStore.setState({ session: null, book: null }))
    expect(container.querySelector('.reader-library-content')?.scrollTop).toBe(240)
  })
})
