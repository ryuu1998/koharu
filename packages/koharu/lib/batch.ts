'use client'

import { Channel } from '@tauri-apps/api/core'
import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { call } from '@/lib/backend'
import {
  commands,
  type BatchChapter,
  type BatchEvent,
  type BatchReport,
  type BatchSource,
  type Stage,
} from '@koharu/bridge/protocol'

type ChapterStatus = 'running' | 'exporting' | 'translated' | 'skipped' | 'failed' | 'stopped'
export type BatchSort = 'added' | 'size' | 'pages'

const chapterNames = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })

export function sortBatchChapters(
  chapters: readonly BatchChapter[],
  sortBy: BatchSort,
  descending: boolean,
): BatchChapter[] {
  return [...chapters].sort((a, b) => {
    const left = sortBy === 'added' ? a.created_at : a[sortBy]
    const right = sortBy === 'added' ? b.created_at : b[sortBy]
    if (left === null && right !== null) return 1
    if (right === null && left !== null) return -1
    const difference = left !== null && right !== null ? left - right : 0
    return (
      (descending ? -difference : difference) ||
      chapterNames.compare(a.name, b.name) ||
      a.path.localeCompare(b.path)
    )
  })
}

interface BatchStore {
  source: BatchSource | null
  output: string | null
  destinations: Record<string, string>
  selected: string[]
  queue: string[]
  runId: string | null
  cardSize: number
  cardLayout: 'overlay' | 'caption'
  sortBy: BatchSort
  sortDescending: boolean
  exported: string[]
  running: boolean
  stopping: boolean
  pauseState: 'playing' | 'pausing' | 'paused'
  activePath: string | null
  progress: Extract<BatchEvent, { event: 'chapter_progress' }> | null
  stages: Partial<Record<Stage, number>>
  completedChapters: number
  totalChapters: number
  statuses: Record<string, ChapterStatus>
  report: BatchReport | null
  setSource: (source: BatchSource) => void
  setOutput: (output: string) => Promise<void>
  select: (paths: string[]) => Promise<void>
  setCardSize: (size: number) => void
  toggleCardLayout: () => void
  setSort: (sortBy: BatchSort) => void
}

export const useBatchStore = create<BatchStore>()(
  persist(
    (set, get) => ({
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
          destinations: Object.fromEntries(
            source.chapters.map((chapter) => [chapter.path, source.default_output]),
          ),
          selected: [],
          exported: source.exported,
          report: null,
          progress: null,
          activePath: null,
          queue: [],
          completedChapters: 0,
          totalChapters: 0,
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
          destinations: Object.fromEntries(
            source.chapters.map((chapter) => [chapter.path, output]),
          ),
          exported,
          statuses: {},
          report: null,
          activePath: null,
          progress: null,
          queue: [],
          completedChapters: 0,
          totalChapters: 0,
          selected: [],
        })
      },
      select: async (selected) => {
        const current = get()
        if (current.running) {
          const paths = [...new Set(selected)].filter(
            (path) =>
              !current.queue.includes(path) &&
              current.source?.chapters.some((chapter) => chapter.path === path && !chapter.error) &&
              !current.exported.includes(path),
          )
          if (!paths.length || current.stopping || !current.runId) return
          await call(
            commands.appendBatch,
            current.runId,
            paths.map((path) => ({ path, output: current.destinations[path] ?? current.output! })),
          )
          if (get().runId === current.runId)
            set((state) => ({ selected: [...new Set([...state.selected, ...paths])] }))
          return
        }
        set({
          selected: [...new Set(selected)],
          queue: [],
          report: null,
          activePath: null,
          progress: null,
        })
      },
      setCardSize: (size) =>
        set({ cardSize: Math.min(160, Math.max(64, Math.round(size / 8) * 8)) }),
      toggleCardLayout: () =>
        set((state) => ({ cardLayout: state.cardLayout === 'overlay' ? 'caption' : 'overlay' })),
      setSort: (sortBy) =>
        set((state) => ({
          sortBy,
          sortDescending: state.sortBy === sortBy ? !state.sortDescending : true,
        })),
    }),
    {
      name: 'koharu-batch-view',
      partialize: ({ cardSize, cardLayout, sortBy, sortDescending }) => ({
        cardSize,
        cardLayout,
        sortBy,
        sortDescending,
      }),
    },
  ),
)

export async function startBatch(): Promise<void> {
  const state = useBatchStore.getState()
  if (state.running || !state.source || !state.output || !state.selected.length) return
  const chapters = state.selected.filter((path) =>
    state.source?.chapters.some((chapter) => chapter.path === path && !chapter.error),
  )
  const id = crypto.randomUUID()
  if (!chapters.length) return
  useBatchStore.setState({
    running: true,
    stopping: false,
    pauseState: 'playing',
    report: null,
    progress: null,
    activePath: null,
    queue: chapters,
    runId: id,
    statuses: {},
    completedChapters: 0,
    totalChapters: chapters.length,
  })
  try {
    const events = new Channel<BatchEvent>((event) => {
      if (useBatchStore.getState().runId !== id) return
      if (event.event === 'queue_changed') {
        useBatchStore.setState({
          queue: event.paths,
          selected: event.paths,
          totalChapters: event.paths.length,
        })
      } else if (event.event === 'started')
        useBatchStore.setState({ totalChapters: event.total_chapters })
      else if (event.event === 'chapter_progress') {
        if (useBatchStore.getState().activePath !== event.path) return
        useBatchStore.setState((state) => ({
          progress: event,
          stages: { ...state.stages, [event.stage]: event.completed_pages },
        }))
      } else if (event.event === 'finished') useBatchStore.setState({ report: event.report })
      else {
        const path = event.path
        if (!path) return
        if (event.event === 'chapter_started') {
          useBatchStore.setState((state) => ({
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
      {
        id,
        chapters: chapters.map((path) => ({
          path,
          output: state.destinations[path] ?? state.output!,
        })),
        jpeg_quality: 95,
        overwrite: false,
      },
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
        runId: null,
        stopping: false,
        pauseState: 'playing',
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

export async function pauseBatch(): Promise<void> {
  const { runId, running, pauseState, stopping } = useBatchStore.getState()
  if (!runId || !running || stopping || pauseState !== 'playing') return
  useBatchStore.setState({ pauseState: 'pausing' })
  try {
    const paused = await call(commands.pauseBatch, runId)
    const current = useBatchStore.getState()
    if (current.runId === runId && current.running && !current.stopping)
      useBatchStore.setState({ pauseState: paused ? 'paused' : 'playing' })
  } catch (error) {
    if (useBatchStore.getState().runId === runId) useBatchStore.setState({ pauseState: 'playing' })
    throw error
  }
}

export async function resumeBatch(): Promise<void> {
  const { runId, running, pauseState } = useBatchStore.getState()
  if (!runId || !running || pauseState !== 'paused') return
  await call(commands.resumeBatch, runId)
  if (useBatchStore.getState().runId === runId) useBatchStore.setState({ pauseState: 'playing' })
}

export async function addBatchSource(
  source: BatchSource,
  output: string,
  expectedRun: string | null,
): Promise<void> {
  const state = useBatchStore.getState()
  if (state.running && state.runId !== expectedRun)
    throw new Error('The batch changed. Import the folder again.')
  const fresh = source.chapters.filter(
    (chapter) => !state.source?.chapters.some((existing) => existing.path === chapter.path),
  )
  const exported = await call(
    commands.getBatchExports,
    fresh.map((chapter) => chapter.path),
    output,
  )
  const paths = fresh
    .filter((chapter) => !chapter.error && !exported.includes(chapter.path))
    .map((chapter) => chapter.path)
  const current = useBatchStore.getState()
  if (current.running && (current.runId !== expectedRun || current.stopping))
    throw new Error('The batch changed. Import the folder again.')
  if (current.running && current.runId && paths.length) {
    await call(
      commands.appendBatch,
      current.runId,
      paths.map((path) => ({ path, output })),
    )
  }
  useBatchStore.setState((state) => ({
    source: state.source
      ? { ...state.source, chapters: [...state.source.chapters, ...fresh] }
      : source,
    destinations: {
      ...state.destinations,
      ...Object.fromEntries(fresh.map((chapter) => [chapter.path, output])),
    },
    exported: [...new Set([...state.exported, ...exported])],
    selected: [...new Set([...state.selected, ...paths])],
    queue: state.running ? state.queue : [],
    report: state.running ? state.report : null,
  }))
}

export async function reorderBatch(path: string, before: string | null): Promise<void> {
  const state = useBatchStore.getState()
  if (state.running) {
    if (
      state.stopping ||
      !state.runId ||
      state.statuses[path] ||
      (before && state.statuses[before])
    )
      return
    await call(commands.reorderBatch, state.runId, path, before)
    return
  }
  const order = state.selected.filter((entry) => entry !== path)
  if (!state.selected.includes(path)) return
  const index = before ? order.indexOf(before) : order.length
  if (index < 0) return
  order.splice(index, 0, path)
  useBatchStore.setState({ selected: order, queue: [] })
}
