import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { BatchPage } from '@/components/batch/BatchPage'
import { useKoharuStore } from '@/lib/store'
import type { BatchSource } from '@koharu/bridge/protocol'

const source: BatchSource = {
  path: 'C:\\Manga Project',
  default_output: 'C:\\Manga Project\\Translated',
  chapters: [
    {
      path: 'C:\\Manga Project\\Chapter 001.cbz',
      name: 'Chapter 001.cbz',
      size: 12_000_000,
      pages: 10,
      thumbnail: null,
      error: null,
    },
    {
      path: 'C:\\Manga Project\\Chapter 002.cbz',
      name: 'Chapter 002.cbz',
      size: 15_000_000,
      pages: 12,
      thumbnail: null,
      error: null,
    },
  ],
}

describe('batch processing workspace', () => {
  it('shows source and designated export folders and supports chapter selection', () => {
    useKoharuStore.setState((state) => ({
      batch: {
        ...state.batch,
        source,
        output: 'D:\\Translated Manga',
        selected: [source.chapters[0].path],
        statuses: { [source.chapters[0].path]: 'idle' },
      },
    }))

    render(<BatchPage />)

    expect(screen.getByText('C:\\Manga Project')).toBeInTheDocument()
    expect(screen.getByText('D:\\Translated Manga')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'Chapter 001.cbz' })).toHaveAttribute(
      'aria-checked',
      'true',
    )

    fireEvent.click(screen.getByRole('checkbox', { name: 'Chapter 002.cbz' }))

    expect(useKoharuStore.getState().batch.selected).toEqual([
      source.chapters[0].path,
      source.chapters[1].path,
    ])
  })

  it('reports active-page and whole-batch progress independently', () => {
    useKoharuStore.setState((state) => ({
      batch: {
        ...state.batch,
        source,
        output: source.default_output,
        selected: source.chapters.map((chapter) => chapter.path),
        running: true,
        statuses: {
          [source.chapters[0].path]: 'completed',
          [source.chapters[1].path]: 'running',
        },
        event: {
          event: 'chapter_progress',
          index: 1,
          completed_pages: 3,
          total_pages: 12,
          completed_steps: 12,
          total_steps: 48,
          stage: 'translation',
        },
      },
    }))

    render(<BatchPage />)

    expect(screen.getByText('3 of 12 pages')).toBeInTheDocument()
    expect(screen.getByText('1 of 2 chapters · 63%')).toBeInTheDocument()
    expect(screen.getByText('Current stage: Translation')).toBeInTheDocument()

    act(() =>
      useKoharuStore.getState().updateBatch({
        running: false,
        event: { event: 'finished', completed: 1, skipped: 0, failed: 0, stopped: true },
        report: { completed: 1, skipped: 0, failures: [], stopped: true },
      }),
    )

    expect(screen.getByText('1 of 2 chapters · 50%')).toBeInTheDocument()
    expect(screen.getByText('Stopped after 1 of 2 chapters.')).toBeInTheDocument()
  })
})
