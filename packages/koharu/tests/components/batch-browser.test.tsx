import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { BatchPage } from '@/components/batch/BatchPage'
import { useBatchStore } from '@/lib/batch'
import { useKoharuStore } from '@/lib/store'
import { commands, type BatchSource } from '@koharu/bridge/protocol'

const source: BatchSource = {
  path: 'C:\\Manga',
  default_output: 'C:\\Manga\\Translated',
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

beforeEach(() => useBatchStore.setState({ source: null, output: null, selected: [] }))

describe('batch chapter browser', () => {
  it('selects healthy chapters and leaves corrupt archives disabled', async () => {
    vi.spyOn(commands, 'browseBatchSource').mockResolvedValue(source)
    render(<BatchPage />)
    fireEvent.click(screen.getByRole('button', { name: 'Choose source folder' }))
    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'chapter1.cbz' })).toBeChecked(),
    )
    expect(screen.getByRole('checkbox', { name: 'broken.cbz' })).toBeDisabled()
    expect(screen.getByText('C:\\Manga\\Translated')).toBeInTheDocument()
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
})
