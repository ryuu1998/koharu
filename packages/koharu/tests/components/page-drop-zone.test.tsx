import { PhysicalPosition } from '@tauri-apps/api/dpi'
import type { DragDropEvent } from '@tauri-apps/api/webview'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PageDropZone } from '@/components/editor/PageDropZone'

const native = vi.hoisted(() => ({
  receive: undefined as undefined | ((event: { payload: DragDropEvent }) => void),
  stop: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', async (original) => ({
  ...(await original<typeof import('@tauri-apps/api/core')>()),
  isTauri: () => true,
}))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (receive: typeof native.receive) => {
      native.receive = receive
      return native.stop
    },
  }),
}))

describe('native page drop zone', () => {
  it('imports a native drop within the target and unregisters on unmount', async () => {
    const onDrop = vi.fn()
    const onBrowse = vi.fn()
    const { unmount } = render(
      <PageDropZone importing={false} onDrop={onDrop} onBrowse={onBrowse} />,
    )
    const zone = screen.getByRole('button', { name: 'Drop pages here' })
    vi.spyOn(zone, 'getBoundingClientRect').mockReturnValue({
      left: 10,
      top: 10,
      right: 100,
      bottom: 100,
    } as DOMRect)
    await waitFor(() => expect(native.receive).toBeDefined())
    act(() =>
      native.receive?.({
        payload: {
          type: 'drop',
          paths: ['chapter.cbz'],
          position: new PhysicalPosition(30, 30),
        },
      }),
    )
    expect(onDrop).toHaveBeenCalledWith(['chapter.cbz'])
    act(() =>
      native.receive?.({
        payload: {
          type: 'drop',
          paths: ['elsewhere.cbz'],
          position: new PhysicalPosition(300, 300),
        },
      }),
    )
    expect(onDrop).toHaveBeenCalledTimes(1)
    fireEvent.click(zone)
    expect(onBrowse).toHaveBeenCalledOnce()
    unmount()
    expect(native.stop).toHaveBeenCalledOnce()
  })

  it('prevents another drop while an import is running', async () => {
    const onDrop = vi.fn()
    render(<PageDropZone importing onDrop={onDrop} onBrowse={vi.fn()} />)
    const zone = screen.getByRole('button', { name: 'Drop pages here' })
    vi.spyOn(zone, 'getBoundingClientRect').mockReturnValue({
      left: 0,
      top: 0,
      right: 100,
      bottom: 100,
    } as DOMRect)
    await waitFor(() => expect(native.receive).toBeDefined())
    act(() =>
      native.receive?.({
        payload: { type: 'drop', paths: ['chapter.cbz'], position: new PhysicalPosition(30, 30) },
      }),
    )
    expect(zone).toBeDisabled()
    expect(onDrop).not.toHaveBeenCalled()
  })
})
