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

type ChapterStatus = 'running' | 'exporting' | 'translated' | 'skipped' | 'failed' | 'stopped'

interface BatchStore {
  source: BatchSource | null
  output: string | null
  selected: string[]
  exported: string[]
  running: boolean
  stopping: boolean
  activeName: string | null
  activePath: string | null
  progress: Extract<BatchEvent, { event: 'chapter_progress' }> | null
  stages: Partial<
    Record<
      Extract<BatchEvent, { event: 'chapter_progress' }>['stage'],
      Extract<BatchEvent, { event: 'chapter_progress' }>
    >
  >
  completedChapters: number
  totalChapters: number
  statuses: Record<string, ChapterStatus>
  report: BatchReport | null
  setSource: (source: BatchSource) => void
  setOutput: (output: string) => Promise<void>
  select: (paths: string[]) => void
}

export const useBatchStore = create<BatchStore>()((set, get) => ({
  source: null,
  output: null,
  selected: [],
  exported: [],
  running: false,
  stopping: false,
  activeName: null,
  activePath: null,
  progress: null,
  stages: {},
  completedChapters: 0,
  totalChapters: 0,
  statuses: {},
  report: null,
  setSource: (source) =>
    set({
      source,
      output: source.default_output,
      selected: source.chapters
        .filter((chapter) => !chapter.error && !source.exported.includes(chapter.path))
        .map((chapter) => chapter.path),
      exported: source.exported,
      report: null,
      progress: null,
      stages: {},
      statuses: {},
    }),
  setOutput: async (output) => {
    const source = get().source
    if (!source) return
    const exported = await call(
      commands.getBatchExports,
      source.chapters.map((chapter) => chapter.path),
      output,
    )
    if (get().source !== source) return
    set({
      output,
      exported,
      statuses: {},
      report: null,
      selected: source.chapters
        .filter((chapter) => !chapter.error && !exported.includes(chapter.path))
        .map((chapter) => chapter.path),
    })
  },
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
    stages: {},
    activeName: null,
    activePath: null,
    statuses: {},
    completedChapters: 0,
    totalChapters: chapters.length,
  })
  try {
    const events = new Channel<BatchEvent>((event) => {
      if (event.event === 'started') useBatchStore.setState({ totalChapters: event.total_chapters })
      else if (event.event === 'chapter_progress')
        useBatchStore.setState((state) => ({
          progress: event,
          stages: { ...state.stages, [event.stage]: event },
        }))
      else if (event.event === 'finished') useBatchStore.setState({ report: event.report })
      else {
        const path = chapters[event.index]
        if (!path) return
        if (event.event === 'chapter_started') {
          useBatchStore.setState((state) => ({
            activeName: event.name,
            activePath: path,
            progress: null,
            stages: {},
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
            exported:
              status === 'translated' ? [...new Set([...state.exported, path])] : state.exported,
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
    useBatchStore.setState((state) => {
      const statuses = { ...state.statuses }
      for (const path of Object.keys(statuses)) {
        if (statuses[path] === 'running' || statuses[path] === 'exporting') {
          statuses[path] = state.report?.stopped ? 'stopped' : 'failed'
        }
      }
      return {
        running: false,
        stopping: false,
        statuses,
        selected: state.selected.filter((path) => !state.exported.includes(path)),
      }
    })
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
