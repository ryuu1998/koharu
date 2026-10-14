import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { BatchPage } from '@/components/batch/BatchPage'
import { useBatchStore } from '@/lib/batch'
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
      thumbnail: null,
      error: null,
    },
    {
      path: 'C:\\Manga\\broken.cbz',
      name: 'broken.cbz',
      pages: 0,
      size: 16,
      thumbnail: null,
      error: 'Invalid archive',
    },
  ],
}

beforeEach(() =>
  useBatchStore.setState({
    source: null,
    output: null,
    selected: [],
    exported: [],
    running: false,
    stopping: false,
    report: null,
    progress: null,
    stages: {},
    activeName: null,
    activePath: null,
    statuses: {},
    completedChapters: 0,
    totalChapters: 0,
  }),
)

describe('batch chapter browser', () => {
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
    expect(useBatchStore.getState().selected).toEqual([source.chapters[0].path])
  })
  it('selects healthy chapters and leaves corrupt archives disabled', async () => {
    vi.spyOn(commands, 'browseBatchSource').mockResolvedValue(source)
    render(<BatchPage />)
    const folders = within(screen.getByRole('group', { name: 'Batch folders' }))
    expect(folders.getByRole('button', { name: 'Choose source folder' })).toHaveTextContent(
      'Source folder',
    )
    expect(folders.getByRole('button', { name: 'Choose export folder' })).toHaveTextContent(
      'Export folder',
    )
    fireEvent.click(
      within(screen.getByRole('group', { name: 'Batch folders' })).getByRole('button', {
        name: 'Choose source folder',
      }),
    )
    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'chapter1.cbz' })).toBeChecked(),
    )
    expect(screen.getByRole('checkbox', { name: 'broken.cbz' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Choose export folder' })).toHaveAttribute(
      'title',
      expect.stringContaining('C:\\Manga\\Translated'),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Clear' }))
    expect(useBatchStore.getState().selected).toEqual([])
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

  it('starts only selected healthy chapters and keeps stop active until completion', async () => {
    useBatchStore.getState().setSource(source)
    let finish: ((report: BatchReport) => void) | undefined
    const run = vi.spyOn(commands, 'processBatch').mockImplementation(async (_request, channel) => {
      channel.onmessage({ event: 'started', total_chapters: 1 })
      channel.onmessage({ event: 'chapter_started', index: 0, name: 'chapter1.cbz', pages: 32 })
      channel.onmessage({
        event: 'chapter_progress',
        index: 0,
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
    render(<BatchPage />)
    fireEvent.click(screen.getByRole('button', { name: 'Translate 1 chapter' }))
    await waitFor(() => expect(run).toHaveBeenCalledOnce())
    expect(run.mock.calls[0][0]).toEqual({
      chapters: ['C:\\Manga\\chapter1.cbz'],
      output: 'C:\\Manga\\Translated',
      jpeg_quality: 95,
      overwrite: false,
    })
    expect(screen.getByRole('checkbox', { name: 'chapter1.cbz' })).toBeDisabled()
    expect(screen.getByRole('progressbar', { name: 'Batch progress' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Collapse active chapter' }))
    expect(screen.getByRole('button', { name: 'Expand active chapter' })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    expect(screen.queryByRole('progressbar', { name: 'Detection' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Expand active chapter' }))
    expect(screen.getByRole('progressbar', { name: 'Detection' })).toBeInTheDocument()
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
