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
export type BatchSort = 'added' | 'size' | 'pages' | 'status'

const chapterNames = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })

export function sortBatchChapters(
  chapters: readonly BatchChapter[],
  sortBy: BatchSort,
  descending: boolean,
  state: Pick<BatchStore, 'statuses' | 'exported' | 'selected'>,
): BatchChapter[] {
  const statusRank = (chapter: BatchChapter) => {
    const status = state.statuses[chapter.path]
    if (status === 'running' || status === 'exporting') return 0
    if (status === 'failed' || chapter.error) return 4
    if (status === 'stopped') return 3
    if (state.exported[chapter.path] || status === 'translated' || status === 'skipped') return 5
    return state.selected.includes(chapter.path) ? 1 : 2
  }
  return [...chapters].sort((a, b) => {
    const left = sortBy === 'status' ? statusRank(a) : sortBy === 'added' ? a.created_at : a[sortBy]
    const right =
      sortBy === 'status' ? statusRank(b) : sortBy === 'added' ? b.created_at : b[sortBy]
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
  exported: Record<string, string>
  running: boolean
  stopping: boolean
  pauseState: 'playing' | 'pausing' | 'paused'
  activePath: string | null
  progress: Extract<BatchEvent, { event: 'chapter_progress' }> | null
  stages: Partial<Record<Stage, number>>
  stageActivity: Partial<Record<Stage, number>>
  rendering: Extract<BatchEvent, { event: 'chapter_rendered' }> | null
  completedChapters: number
  totalChapters: number
  statuses: Record<string, ChapterStatus>
  errors: Record<string, string>
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
      exported: {},
      running: false,
      stopping: false,
      pauseState: 'playing',
      activePath: null,
      progress: null,
      stages: {},
      stageActivity: {},
      rendering: null,
      completedChapters: 0,
      totalChapters: 0,
      statuses: {},
      errors: {},
      report: null,
      setSource: (source) =>
        set({
          source,
          output: source.default_output,
          destinations: Object.fromEntries(
            source.chapters.map((chapter) => [chapter.path, source.default_output]),
          ),
          selected: [],
          exported: Object.fromEntries(
            source.exported.map(({ source, output }) => [source, output]),
          ),
          report: null,
          progress: null,
          stages: {},
          stageActivity: {},
          rendering: null,
          activePath: null,
          queue: [],
          completedChapters: 0,
          totalChapters: 0,
          statuses: {},
          errors: {},
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
          exported: Object.fromEntries(exported.map(({ source, output }) => [source, output])),
          statuses: {},
          errors: {},
          report: null,
          activePath: null,
          progress: null,
          stages: {},
          stageActivity: {},
          rendering: null,
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
              current.source?.chapters.some((chapter) => chapter.path === path && !chapter.error),
          )
          if (!paths.length || current.stopping || !current.runId) return
          await call(
            commands.appendBatch,
            current.runId,
            paths.map((path) => ({
              path,
              output: current.destinations[path] ?? current.output!,
              replacement: current.exported[path] ?? null,
            })),
          )
          if (get().runId === current.runId)
            set((state) => ({ selected: [...new Set([...state.selected, ...paths])] }))
          return
        }
        set({
          selected: [...new Set(selected)],
          queue: [],
          statuses: {},
          report: null,
          activePath: null,
          progress: null,
          stages: {},
          stageActivity: {},
          rendering: null,
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
    stages: {},
    stageActivity: {},
    rendering: null,
    activePath: null,
    queue: chapters,
    runId: id,
    statuses: {},
    errors: {},
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
      } else if (event.event === 'chapter_queued') {
        useBatchStore.setState((state) => {
          const statuses = { ...state.statuses }
          const errors = { ...state.errors }
          delete statuses[event.path]
          delete errors[event.path]
          return { statuses, errors, completedChapters: Math.max(0, state.completedChapters - 1) }
        })
      } else if (event.event === 'started')
        useBatchStore.setState({ totalChapters: event.total_chapters })
      else if (event.event === 'chapter_progress') {
        if (useBatchStore.getState().activePath !== event.path) return
        useBatchStore.setState((state) => ({
          progress: event,
          stages: { ...state.stages, [event.stage]: event.completed_pages },
          stageActivity: { ...state.stageActivity, [event.stage]: event.active_pages },
        }))
      } else if (event.event === 'chapter_rendered') {
        if (useBatchStore.getState().activePath === event.path)
          useBatchStore.setState({ rendering: event })
      } else if (event.event === 'finished') useBatchStore.setState({ report: event.report })
      else {
        const path = event.path
        if (!path) return
        if (event.event === 'chapter_started') {
          useBatchStore.setState((state) => ({
            activePath: path,
            progress: null,
            rendering: null,
            stages: {},
            stageActivity: {},
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
            errors:
              event.event === 'chapter_failed'
                ? { ...state.errors, [path]: event.error }
                : state.errors,
            exported:
              event.event === 'chapter_finished' || event.event === 'chapter_skipped'
                ? { ...state.exported, [path]: event.output }
                : state.exported,
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
          replacement: state.exported[path] ?? null,
        })),
        jpeg_quality: 95,
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
        stageActivity: {},
        statuses,
        selected: state.selected.filter(
          (path) =>
            !state.exported[path] || statuses[path] === 'failed' || statuses[path] === 'stopped',
        ),
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
    .filter((chapter) => !chapter.error && !exported.some((entry) => entry.source === chapter.path))
    .map((chapter) => chapter.path)
  const current = useBatchStore.getState()
  if (current.running && (current.runId !== expectedRun || current.stopping))
    throw new Error('The batch changed. Import the folder again.')
  if (current.running && current.runId && paths.length) {
    await call(
      commands.appendBatch,
      current.runId,
      paths.map((path) => ({ path, output, replacement: null })),
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
    exported: {
      ...state.exported,
      ...Object.fromEntries(exported.map(({ source, output }) => [source, output])),
    },
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

export async function removeBatch(path: string): Promise<void> {
  const state = useBatchStore.getState()
  if (state.stopping) return
  if (state.running) {
    if (!state.runId || state.statuses[path] || state.activePath === path) return
    await call(commands.removeBatch, state.runId, path)
    return
  }
  useBatchStore.setState((state) => ({
    selected: state.selected.filter((entry) => entry !== path),
    queue: state.queue.filter((entry) => entry !== path),
  }))
}

export async function retryBatch(path: string): Promise<void> {
  const state = useBatchStore.getState()
  if (state.stopping || state.statuses[path] !== 'failed') return
  if (state.running) {
    if (state.runId) await call(commands.retryBatch, state.runId, path)
    return
  }
  const statuses = { ...state.statuses }
  const errors = { ...state.errors }
  delete statuses[path]
  delete errors[path]
  useBatchStore.setState({
    selected: [...state.selected.filter((entry) => entry !== path), path],
    queue: [],
    statuses,
    errors,
    report: null,
    completedChapters: Math.max(0, state.completedChapters - 1),
  })
}
