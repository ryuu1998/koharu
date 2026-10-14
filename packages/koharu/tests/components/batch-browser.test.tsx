import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { BatchPage } from '@/components/batch/BatchPage'
import { addBatchSource, sortBatchChapters, useBatchStore } from '@/lib/batch'
import { useKoharuStore } from '@/lib/store'
import { commands, type BatchSource } from '@koharu/bridge/protocol'
import type { BatchEvent, BatchReport } from '@koharu/bridge/protocol'

vi.mock('@tauri-apps/api/core', async (original) => ({
  ...(await original<typeof import('@tauri-apps/api/core')>()),
  Channel: class {
    constructor(public onmessage: (event: BatchEvent) => void) {}
  },
}))

const source: BatchSource = {
  path: 'C:\\Manga',
  default_output: 'C:\\Manga\\Translated',
  exported: [],
  chapters: [
    {
      path: 'C:\\Manga\\chapter1.cbz',
      name: 'chapter1.cbz',
      pages: 32,
      size: 1048576,
      created_at: 1700000000000,
      thumbnail: null,
      error: null,
    },
    {
      path: 'C:\\Manga\\broken.cbz',
      name: 'broken.cbz',
      pages: 0,
      size: 16,
      created_at: null,
      thumbnail: null,
      error: 'Invalid archive',
    },
  ],
}

beforeEach(() => {
  HTMLElement.prototype.scrollTo = vi.fn()
  useBatchStore.setState({
    source: null,
    output: null,
    destinations: {},
    selected: [],
    queue: [],
    runId: null,
    cardSize: 96,
    cardLayout: 'overlay',
    sortBy: 'added',
    sortDescending: true,
    exported: [],
    running: false,
    stopping: false,
    pauseState: 'playing',
    stages: {},
    report: null,
    progress: null,
    activePath: null,
    statuses: {},
    completedChapters: 0,
    totalChapters: 0,
  })
})

const sortableSource: BatchSource = {
  ...source,
  chapters: [
    source.chapters[0],
    {
      ...source.chapters[0],
      path: 'chapter2.cbz',
      name: 'chapter2.cbz',
      pages: 12,
      size: 2097152,
      created_at: 1700000001000,
    },
    {
      ...source.chapters[0],
      path: 'chapter10.cbz',
      name: 'chapter10.cbz',
      size: 3145728,
      created_at: null,
    },
  ],
}

describe('batch chapter browser', () => {
  it('switches compact layouts without hiding chapter names or page counts', () => {
    useBatchStore.getState().setSource(source)
    render(<BatchPage />)
    const name = screen.getByRole('checkbox', { name: 'chapter1.cbz' })
    expect(screen.getByLabelText('32 pages')).toHaveTextContent('32P')
    fireEvent.click(screen.getByRole('button', { name: 'Show titles below covers' }))
    expect(useBatchStore.getState().cardLayout).toBe('caption')
    expect(name).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Show titles on covers' }))
    expect(useBatchStore.getState().cardLayout).toBe('overlay')
  })

  it('keeps unknown dates last in both directions and uses natural names for tied values', () => {
    const names = (sort: 'added' | 'pages', descending: boolean) =>
      sortBatchChapters(sortableSource.chapters, sort, descending).map((c) => c.name)
    expect(names('added', true)).toEqual(['chapter2.cbz', 'chapter1.cbz', 'chapter10.cbz'])
    expect(names('added', false)).toEqual(['chapter1.cbz', 'chapter2.cbz', 'chapter10.cbz'])
    expect(names('pages', true)).toEqual(['chapter1.cbz', 'chapter10.cbz', 'chapter2.cbz'])
    expect(sortableSource.chapters.map((c) => c.name)).toEqual([
      'chapter1.cbz',
      'chapter2.cbz',
      'chapter10.cbz',
    ])
  })

  it('resizes cards with the keyboard and reverses the selected sorting criterion', async () => {
    useBatchStore.getState().setSource(sortableSource)
    render(<BatchPage />)
    await act(async () => {
      fireEvent.keyDown(screen.getByLabelText('Card size', { selector: 'input' }), {
        key: 'ArrowRight',
      })
    })
    expect(useBatchStore.getState().cardSize).toBe(104)
    fireEvent.click(screen.getByRole('button', { name: 'Choose sorting' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Page count' }))
    expect(useBatchStore.getState().sortBy).toBe('pages')
    expect(useBatchStore.getState().sortDescending).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Most pages first. Reverse order' }))
    expect(useBatchStore.getState().sortDescending).toBe(false)
    expect(
      screen.getByRole('button', { name: 'Fewest pages first. Reverse order' }),
    ).toBeInTheDocument()
  })

  it('preserves selection order independently of sorting and uses stable progress identities', async () => {
    useBatchStore.getState().setSource(sortableSource)
    const firstPath = 'chapter2.cbz'
    useBatchStore.getState().select([firstPath, source.chapters[0].path, 'chapter10.cbz'])
    useBatchStore.getState().setSort('pages')
    let events: { onmessage: (event: BatchEvent) => void } | undefined
    let finish: ((report: BatchReport) => void) | undefined
    const run = vi.spyOn(commands, 'processBatch').mockImplementation(async (_request, channel) => {
      events = channel
      channel.onmessage({ event: 'chapter_started', path: firstPath, name: firstPath, pages: 12 })
      return new Promise<BatchReport>((resolve) => {
        finish = resolve
      })
    })
    render(<BatchPage />)
    fireEvent.click(screen.getByRole('button', { name: 'Translate 3 chapters' }))
    await waitFor(() => expect(run).toHaveBeenCalledOnce())
    const order = [firstPath, source.chapters[0].path, 'chapter10.cbz']
    expect(run.mock.calls[0][0].chapters.map((entry) => entry.path)).toEqual(order)
    const queue = within(screen.getByRole('complementary', { name: 'Queue' }))
    const first = queue.getByRole('listitem', { name: firstPath })
    fireEvent.click(screen.getByRole('button', { name: 'Most pages first. Reverse order' }))
    expect(useBatchStore.getState().queue).toEqual(order)
    act(() => {
      events?.onmessage({
        event: 'chapter_progress',
        path: firstPath,
        completed_pages: 12,
        total_pages: 12,
        completed_steps: 48,
        total_steps: 48,
        stage: 'translation',
      })
      events?.onmessage({ event: 'chapter_exporting', path: firstPath })
    })
    expect(screen.getByRole('progressbar', { name: 'Chapter progress' })).toHaveAttribute(
      'aria-valuenow',
      '99',
    )
    expect(useBatchStore.getState().exported).toEqual([])
    fireEvent.click(screen.getByRole('button', { name: 'Collapse queue' }))
    act(() => {
      events?.onmessage({
        event: 'chapter_finished',
        path: firstPath,
        output: 'output/chapter2.cbz',
      })
      events?.onmessage({
        event: 'chapter_started',
        path: 'chapter10.cbz',
        name: 'chapter10.cbz',
        pages: 32,
      })
    })
    expect(screen.getByRole('button', { name: 'Expand queue' })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    fireEvent.click(screen.getByRole('button', { name: 'Expand queue' }))
    expect(queue.getByRole('listitem', { name: firstPath })).toBe(first)
    expect(within(first).getByText('Translated')).toBeInTheDocument()
    await act(async () => {
      finish?.({ completed: 1, skipped: 0, failures: [], stopped: true })
    })
  })
  it('marks previous exports and updates selection for a different output folder', async () => {
    useBatchStore.getState().setSource({ ...source, exported: [source.chapters[0].path] })
    const check = vi.spyOn(commands, 'getBatchExports').mockResolvedValue([])
    render(<BatchPage />)
    expect(screen.getByText('Translated')).toHaveAttribute('title', 'Already translated')
    expect(screen.getByRole('checkbox', { name: 'chapter1.cbz' })).toBeDisabled()
    expect(useBatchStore.getState().selected).toEqual([])
    await useBatchStore.getState().setOutput('D:\\Translated')
    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'chapter1.cbz' })).toBeEnabled(),
    )
    expect(check).toHaveBeenCalledWith(
      source.chapters.map((chapter) => chapter.path),
      'D:\\Translated',
    )
    expect(useBatchStore.getState().selected).toEqual([])
  })
  it('selects healthy chapters and leaves corrupt archives disabled', async () => {
    vi.spyOn(commands, 'browseBatchSource').mockResolvedValue(source)
    render(<BatchPage />)
    const folders = within(screen.getByRole('group', { name: 'Batch folders' }))
    expect(folders.getByRole('button', { name: 'Choose source folder' })).toHaveTextContent(
      'Import folder',
    )
    expect(folders.getByRole('button', { name: 'Choose export folder' })).toHaveTextContent(
      'Output folder',
    )
    fireEvent.click(
      within(screen.getByRole('group', { name: 'Batch folders' })).getByRole('button', {
        name: 'Choose source folder',
      }),
    )
    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'chapter1.cbz' })).not.toBeChecked(),
    )
    expect(screen.getByRole('checkbox', { name: 'broken.cbz' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Choose export folder' })).toHaveAttribute(
      'title',
      expect.stringContaining('C:\\Manga\\Translated'),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Select all' }))
    expect(useBatchStore.getState().selected).toEqual(['C:\\Manga\\chapter1.cbz'])
  })

  it('returns to Batch after settings without losing folder selection', () => {
    useBatchStore.getState().setSource(source)
    useKoharuStore.getState().setWorkspace('batch')
    render(<BatchPage />)
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    expect(useKoharuStore.getState().workspace).toBe('settings')
    useKoharuStore.getState().closeSettings()
    expect(useKoharuStore.getState().workspace).toBe('batch')
    expect(useBatchStore.getState().source).toEqual(source)
  })

  it('appends selected chapters and new folders during a run, and drags only waiting entries', async () => {
    useBatchStore.getState().setSource(sortableSource)
    await useBatchStore.getState().select([source.chapters[0].path, 'chapter2.cbz'])
    let events: { onmessage: (event: BatchEvent) => void } | undefined
    let finish: ((report: BatchReport) => void) | undefined
    vi.spyOn(commands, 'processBatch').mockImplementation(async (_request, channel) => {
      events = channel
      channel.onmessage({
        event: 'chapter_started',
        path: source.chapters[0].path,
        name: 'chapter1.cbz',
        pages: 32,
      })
      return new Promise<BatchReport>((resolve) => {
        finish = resolve
      })
    })
    const append = vi.spyOn(commands, 'appendBatch').mockImplementation(async (_id, entries) => {
      events?.onmessage({
        event: 'queue_changed',
        paths: [...useBatchStore.getState().queue, ...entries.map((entry) => entry.path)],
      })
      return null
    })
    const reorder = vi.spyOn(commands, 'reorderBatch').mockResolvedValue(null)
    vi.spyOn(commands, 'getBatchExports').mockResolvedValue([])
    render(<BatchPage />)
    fireEvent.click(screen.getByRole('button', { name: 'Translate 2 chapters' }))
    await waitFor(() => expect(useBatchStore.getState().running).toBe(true))
    fireEvent.click(screen.getByRole('checkbox', { name: 'chapter10.cbz' }))
    await waitFor(() => expect(append).toHaveBeenCalledOnce())
    const queue = within(screen.getByRole('complementary', { name: 'Queue' }))
    const active = queue.getByRole('listitem', { name: 'chapter1.cbz' })
    const second = queue.getByRole('listitem', { name: 'chapter2.cbz' })
    const third = queue.getByRole('listitem', { name: 'chapter10.cbz' })
    expect(active).toHaveAttribute('draggable', 'false')
    expect(third).toHaveAttribute('draggable', 'true')
    const dataTransfer = { setData: vi.fn(), effectAllowed: '', dropEffect: '' }
    fireEvent.dragStart(third, { dataTransfer })
    fireEvent.dragOver(second, { dataTransfer, clientY: -10 })
    fireEvent.drop(second, { dataTransfer, clientY: -10 })
    await waitFor(() =>
      expect(reorder).toHaveBeenCalledWith(
        useBatchStore.getState().runId,
        'chapter10.cbz',
        'chapter2.cbz',
      ),
    )
    const imported = {
      ...source,
      path: 'D:\\More',
      default_output: 'D:\\More\\Translated',
      chapters: [{ ...source.chapters[0], path: 'D:\\More\\chapter20.cbz', name: 'chapter20.cbz' }],
    }
    await act(async () => {
      await addBatchSource(imported, imported.default_output, useBatchStore.getState().runId)
    })
    expect(useBatchStore.getState().activePath).toBe(source.chapters[0].path)
    expect(useBatchStore.getState().output).toBe(source.default_output)
    expect(useBatchStore.getState().destinations[imported.chapters[0].path]).toBe(
      imported.default_output,
    )
    expect(useBatchStore.getState().queue.at(-1)).toBe(imported.chapters[0].path)
    await act(async () => {
      await addBatchSource(imported, imported.default_output, useBatchStore.getState().runId)
    })
    expect(append).toHaveBeenCalledTimes(2)
    await act(async () => {
      finish?.({ completed: 0, skipped: 0, failures: [], stopped: true })
    })
  })

  it('starts only selected healthy chapters and keeps stop active until completion', async () => {
    useBatchStore.getState().setSource(source)
    useBatchStore.getState().select([source.chapters[0].path])
    let finish: ((report: BatchReport) => void) | undefined
    const run = vi.spyOn(commands, 'processBatch').mockImplementation(async (_request, channel) => {
      channel.onmessage({ event: 'started', total_chapters: 1 })
      channel.onmessage({
        event: 'chapter_started',
        path: source.chapters[0].path,
        name: 'chapter1.cbz',
        pages: 32,
      })
      channel.onmessage({
        event: 'chapter_progress',
        path: source.chapters[0].path,
        completed_pages: 8,
        total_pages: 32,
        completed_steps: 8,
        total_steps: 128,
        stage: 'detection',
      })
      return new Promise<BatchReport>((resolve) => {
        finish = resolve
      })
    })
    const stop = vi.spyOn(commands, 'stopBatch').mockResolvedValue(null)
    const pause = vi.spyOn(commands, 'pauseBatch').mockResolvedValue(true)
    const resume = vi.spyOn(commands, 'resumeBatch').mockResolvedValue(null)
    render(<BatchPage />)
    fireEvent.click(screen.getByRole('button', { name: 'Translate 1 chapter' }))
    await waitFor(() => expect(run).toHaveBeenCalledOnce())
    expect(run.mock.calls[0][0]).toEqual({
      id: expect.any(String),
      chapters: [{ path: source.chapters[0].path, output: source.default_output }],
      jpeg_quality: 95,
      overwrite: false,
    })
    expect(screen.getByRole('checkbox', { name: 'chapter1.cbz' })).toBeDisabled()
    expect(screen.getByRole('progressbar', { name: 'Chapter progress' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Collapse queue' }))
    expect(screen.getByRole('button', { name: 'Expand queue' })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    expect(screen.queryByRole('progressbar', { name: 'Chapter progress' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Expand queue' }))
    expect(screen.getByRole('progressbar', { name: 'Chapter progress' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Pause' }))
    await waitFor(() => expect(pause).toHaveBeenCalledOnce())
    await waitFor(() => expect(screen.getByRole('button', { name: 'Resume' })).toBeEnabled())
    expect(useBatchStore.getState().stages.detection).toBe(8)
    expect(screen.getByRole('progressbar', { name: 'Detection' })).toHaveAttribute(
      'aria-valuenow',
      '25',
    )
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    await waitFor(() => expect(resume).toHaveBeenCalledOnce())
    await waitFor(() => expect(screen.getByRole('button', { name: 'Pause' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await waitFor(() => expect(stop).toHaveBeenCalledOnce())
    expect(screen.getByRole('button', { name: 'Stopping…' })).toBeDisabled()
    finish?.({ completed: 0, skipped: 0, failures: [], stopped: true })
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Translate 1 chapter' })).toBeEnabled(),
    )
    expect(useBatchStore.getState().report?.stopped).toBe(true)
    expect(useBatchStore.getState().statuses['C:\\Manga\\chapter1.cbz']).toBe('stopped')
  })
})
