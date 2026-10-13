'use client'

import { Channel } from '@tauri-apps/api/core'
import { create } from 'zustand'

import { call } from '@/lib/backend'
import {
  commands,
  type BatchEvent,
  type BatchReport,
  type BatchSource,
} from '@koharu/bridge/protocol'

type ChapterStatus = 'running' | 'exporting' | 'translated' | 'skipped' | 'failed'

interface BatchStore {
  source: BatchSource | null
  output: string | null
  selected: string[]
  running: boolean
  stopping: boolean
  activeName: string | null
  activePath: string | null
  progress: Extract<BatchEvent, { event: 'chapter_progress' }> | null
  completedChapters: number
  totalChapters: number
  statuses: Record<string, ChapterStatus>
  report: BatchReport | null
  setSource: (source: BatchSource) => void
  setOutput: (output: string) => void
  select: (paths: string[]) => void
}

export const useBatchStore = create<BatchStore>()((set) => ({
  source: null,
  output: null,
  selected: [],
  running: false,
  stopping: false,
  activeName: null,
  activePath: null,
  progress: null,
  completedChapters: 0,
  totalChapters: 0,
  statuses: {},
  report: null,
  setSource: (source) =>
    set({
      source,
      output: source.default_output,
      selected: source.chapters.filter((chapter) => !chapter.error).map((chapter) => chapter.path),
      report: null,
      progress: null,
      statuses: {},
    }),
  setOutput: (output) => set({ output }),
  select: (selected) => set({ selected: [...new Set(selected)] }),
}))

export async function startBatch(): Promise<void> {
  const state = useBatchStore.getState()
  if (state.running || !state.source || !state.output || !state.selected.length) return
  const chapters = state.source.chapters
    .filter((chapter) => !chapter.error && state.selected.includes(chapter.path))
    .map((chapter) => chapter.path)
  if (!chapters.length) return
  useBatchStore.setState({
    running: true,
    stopping: false,
    report: null,
    progress: null,
    activeName: null,
    activePath: null,
    statuses: {},
    completedChapters: 0,
    totalChapters: chapters.length,
  })
  try {
    const events = new Channel<BatchEvent>((event) => {
      if (event.event === 'started') useBatchStore.setState({ totalChapters: event.total_chapters })
      else if (event.event === 'chapter_progress') useBatchStore.setState({ progress: event })
      else if (event.event === 'finished') useBatchStore.setState({ report: event.report })
      else {
        const path = chapters[event.index]
        if (!path) return
        if (event.event === 'chapter_started') {
          useBatchStore.setState((state) => ({
            activeName: event.name,
            activePath: path,
            progress: null,
            statuses: { ...state.statuses, [path]: 'running' },
          }))
        } else if (event.event === 'chapter_exporting') {
          useBatchStore.setState((state) => ({
            statuses: { ...state.statuses, [path]: 'exporting' },
          }))
        } else {
          const status =
            event.event === 'chapter_finished'
              ? 'translated'
              : event.event === 'chapter_skipped'
                ? 'skipped'
                : 'failed'
          useBatchStore.setState((state) => ({
            completedChapters: state.completedChapters + 1,
            statuses: { ...state.statuses, [path]: status },
          }))
        }
      }
    })
    const report = await call(
      commands.processBatch,
      { chapters, output: state.output, jpeg_quality: 95, overwrite: false },
      events,
    )
    useBatchStore.setState({ report })
  } finally {
    useBatchStore.setState({ running: false, stopping: false })
  }
}

export async function stopBatch(): Promise<void> {
  const state = useBatchStore.getState()
  if (!state.running || state.stopping) return
  useBatchStore.setState({ stopping: true })
  try {
    await call(commands.stopBatch)
  } catch (error) {
    useBatchStore.setState({ stopping: false })
    throw error
  }
}
