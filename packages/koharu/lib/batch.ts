'use client'

import { create } from 'zustand'

import type { BatchSource } from '@koharu/bridge/protocol'

interface BatchStore {
  source: BatchSource | null
  output: string | null
  selected: string[]
  setSource: (source: BatchSource) => void
  setOutput: (output: string) => void
  select: (paths: string[]) => void
}

export const useBatchStore = create<BatchStore>()((set) => ({
  source: null,
  output: null,
  selected: [],
  setSource: (source) =>
    set({
      source,
      output: source.default_output,
      selected: source.chapters.filter((chapter) => !chapter.error).map((chapter) => chapter.path),
    }),
  setOutput: (output) => set({ output }),
  select: (selected) => set({ selected: [...new Set(selected)] }),
}))
