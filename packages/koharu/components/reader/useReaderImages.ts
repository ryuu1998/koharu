'use client'

import { useCallback, useEffect, useRef, useState } from 'react'

import { commands, type ReaderSession } from '@koharu/bridge/protocol'

const DECODED_BUDGET = 96 * 1024 * 1024

export function useReaderImages(session: ReaderSession, page: number, visiblePages = 1) {
  const cache = useRef(new Map<number, string>())
  const pending = useRef(new Set<number>())
  const dimensions = useRef(new Map<number, number>())
  const blocked = useRef(new Set<number>())
  const current = useRef({ session: session.id, page, visiblePages })
  const generation = useRef({ active: true })
  const [images, setImages] = useState<Record<number, string>>({})
  const [errors, setErrors] = useState<Record<number, string>>({})
  const [sizeVersion, setSizeVersion] = useState(0)
  current.current = { session: session.id, page, visiblePages }

  const recordDimensions = useCallback((index: number, width: number, height: number) => {
    const bytes = width * height * 4
    if (dimensions.current.get(index) === bytes) return
    dimensions.current.set(index, bytes)
    const { page: active, visiblePages: count } = current.current
    const required = new Set(Array.from({ length: count }, (_, offset) => active + offset))
    let total = [...cache.current.keys()].reduce(
      (sum, item) => sum + (dimensions.current.get(item) ?? bytes),
      0,
    )
    for (const candidate of [...cache.current.keys()].sort(
      (a, b) => Math.abs(b - active) - Math.abs(a - active),
    )) {
      if (total <= DECODED_BUDGET) break
      if (required.has(candidate)) continue
      total -= dimensions.current.get(candidate) ?? bytes
      URL.revokeObjectURL(cache.current.get(candidate)!)
      cache.current.delete(candidate)
      blocked.current.add(candidate)
    }
    setImages(Object.fromEntries(cache.current))
    setSizeVersion((version) => version + 1)
  }, [])

  useEffect(() => {
    const token = { active: true }
    generation.current = token
    const urls = new Map<number, string>()
    cache.current = urls
    pending.current = new Set()
    dimensions.current = new Map()
    blocked.current = new Set()
    setImages({})
    setErrors({})
    return () => {
      token.active = false
      urls.forEach((url) => URL.revokeObjectURL(url))
      urls.clear()
    }
  }, [session.id])

  useEffect(() => {
    blocked.current.clear()
  }, [page])

  useEffect(() => {
    const requestGeneration = generation.current
    const requests = pending.current
    const urls = cache.current
    const required = Array.from({ length: visiblePages }, (_, offset) => page + offset).filter(
      (index) => index < session.pages.length,
    )
    const candidates = [...new Set([...required, page + 1, page - 1])]
      .filter((index) => index >= 0 && index < session.pages.length)
      .slice(0, 3)
    for (const [index, url] of urls) {
      if (!candidates.includes(index)) {
        URL.revokeObjectURL(url)
        urls.delete(index)
      }
    }
    setImages(Object.fromEntries(urls))
    for (const index of candidates) {
      if (urls.has(index) || requests.has(index)) continue
      if (!required.includes(index)) {
        const estimate = dimensions.current.get(page)
        if (!estimate || blocked.current.has(index)) continue
        const total = [...new Set([...urls.keys(), ...requests])].reduce(
          (sum, item) => sum + (dimensions.current.get(item) ?? estimate),
          0,
        )
        if (total + (dimensions.current.get(index) ?? estimate) > DECODED_BUDGET) continue
      }
      requests.add(index)
      void commands
        .getReaderPage(session.id, index)
        .then((bytes) => {
          const active = current.current
          if (
            !requestGeneration.active ||
            requestGeneration !== generation.current ||
            active.session !== session.id ||
            index < active.page - 1 ||
            index > active.page + Math.max(1, active.visiblePages - 1)
          )
            return
          const data = bytes instanceof ArrayBuffer ? bytes : new Uint8Array(bytes)
          const url = URL.createObjectURL(new Blob([data], { type: session.pages[index].mime }))
          urls.set(index, url)
          setImages(Object.fromEntries(urls))
        })
        .catch((error: unknown) => {
          if (
            requestGeneration.active &&
            requestGeneration === generation.current &&
            current.current.session === session.id
          )
            setErrors((previous) => ({ ...previous, [index]: String(error) }))
        })
        .finally(() => requests.delete(index))
    }
  }, [session.id, session.pages, page, visiblePages, sizeVersion])

  return { images, errors, recordDimensions }
}
