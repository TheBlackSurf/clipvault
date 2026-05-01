import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Clipboard,
  Copy,
  Eraser,
  ExternalLink,
  Globe,
  Image as ImageIcon,
  Link,
  Pause,
  Play,
  Search,
  Settings as SettingsIcon,
  Trash2,
  Type,
} from 'lucide-react'
import './App.css'

type ClipboardKind = 'text' | 'link' | 'image'

type ClipboardItem = {
  id: number
  kind: ClipboardKind
  content: string
  url?: string | null
  domain?: string | null
  title?: string | null
  sourceApp?: string | null
  sourceUrl?: string | null
  sourceTitle?: string | null
  sourceDomain?: string | null
  createdAt: number
  thumbDataUrl?: string | null
}

type Settings = {
  retentionDays: number | null
  maxItems: number
  shortcut: string
  paused: boolean
  hideAfterCopy: boolean
}

const DEFAULT_SETTINGS: Settings = {
  retentionDays: 7,
  maxItems: 500,
  shortcut: 'CommandOrControl+Shift+S',
  paused: false,
  hideAfterCopy: true,
}

const kindLabel: Record<ClipboardKind, string> = {
  text: 'Tekst',
  link: 'Link',
  image: 'Obraz',
}

function App() {
  const [items, setItems] = useState<ClipboardItem[]>([])
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS)
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState(0)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [message, setMessage] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
  const resultsRef = useRef<HTMLElement>(null)

  const focusSearch = useCallback(() => {
    requestAnimationFrame(() => {
      searchRef.current?.focus()
      searchRef.current?.select()
    })
  }, [])

  const loadItems = useCallback(async (nextQuery: string) => {
    const result = await invoke<ClipboardItem[]>('get_items', { query: nextQuery })
    setItems(result)
    setSelected((current) => Math.min(current, Math.max(result.length - 1, 0)))
  }, [])

  const loadSettings = useCallback(async () => {
    const result = await invoke<Settings>('get_settings')
    setSettings(result)
  }, [])

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      loadSettings()
      loadItems('')
      focusSearch()
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [focusSearch, loadItems, loadSettings])

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      loadItems(query)
    }, 80)
    return () => window.clearTimeout(timeout)
  }, [loadItems, query])

  useEffect(() => {
    const unlisteners: Array<() => void> = []
    listen('clipboard-updated', () => loadItems(query)).then((unlisten) =>
      unlisteners.push(unlisten),
    )
    listen<Settings>('settings-updated', (event) => setSettings(event.payload)).then((unlisten) =>
      unlisteners.push(unlisten),
    )
    listen('focus-search', () => {
      setSelected(0)
      requestAnimationFrame(() => {
        resultsRef.current?.scrollTo({ top: 0 })
        focusSearch()
      })
    }).then((unlisten) => unlisteners.push(unlisten))
    return () => {
      unlisteners.forEach((unlisten) => unlisten())
    }
  }, [focusSearch, loadItems, query])

  useEffect(() => {
    const selectedRow = document.querySelector('.history-row.selected')
    selectedRow?.scrollIntoView({ block: 'nearest' })
  }, [selected, items])

  const selectedItem = items[selected]

  async function copyItem(item: ClipboardItem) {
    await invoke('copy_item', { id: item.id })
    setMessage('Skopiowano do schowka')
    window.setTimeout(() => setMessage(''), 1400)
  }

  async function openSourceUrl(item: ClipboardItem) {
    await invoke('open_source_url', { id: item.id })
  }

  async function openItemUrl(item: ClipboardItem) {
    try {
      await invoke('open_item_url', { id: item.id })
    } catch {
      setMessage('Ten wpis nie ma linku do otwarcia')
      window.setTimeout(() => setMessage(''), 1400)
    }
  }

  async function deleteItem(item: ClipboardItem) {
    await invoke('delete_item', { id: item.id })
    await loadItems(query)
  }

  async function clearHistory() {
    await invoke('clear_history')
    setItems([])
    setSelected(0)
  }

  async function updatePause(paused: boolean) {
    const result = await invoke<Settings>('toggle_pause', { paused })
    setSettings(result)
  }

  async function saveSettings() {
    const result = await invoke<Settings>('save_settings', { settings })
    setSettings(result)
    setSettingsOpen(false)
    setMessage('Zapisano ustawienia')
    window.setTimeout(() => setMessage(''), 1400)
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      setSelected((value) => Math.min(value + 1, Math.max(items.length - 1, 0)))
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault()
      setSelected((value) => Math.max(value - 1, 0))
    }
    if (event.key === 'Enter' && selectedItem) {
      event.preventDefault()
      if (event.metaKey || event.ctrlKey) {
        openItemUrl(selectedItem)
        return
      }
      copyItem(selectedItem)
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      setSettingsOpen(false)
      setQuery('')
      invoke('hide_window')
    }
  }

  const retentionValue = settings.retentionDays === null ? 'none' : String(settings.retentionDays)
  const visibleStatus = useMemo(() => {
    if (settings.paused) return 'Monitoring wstrzymany'
    if (items.length === 0) return 'Czekam na pierwsze kopiowanie'
    return `${items.length} ostatnich wpisów`
  }, [items.length, settings.paused])

  return (
    <main className="app-shell" onKeyDown={onKeyDown}>
      <header className="topbar">
        <div className="brand">
          <Clipboard size={22} aria-hidden="true" />
          <div>
            <h1>ClipVault</h1>
            <p>{visibleStatus}</p>
          </div>
        </div>
        <div className="toolbar" aria-label="Akcje">
          <button
            className={settings.paused ? 'icon-button active' : 'icon-button'}
            type="button"
            title={settings.paused ? 'Wznów monitoring' : 'Wstrzymaj monitoring'}
            onClick={() => updatePause(!settings.paused)}
          >
            {settings.paused ? <Play size={18} /> : <Pause size={18} />}
          </button>
          <button
            className="icon-button"
            type="button"
            title="Ustawienia"
            onClick={() => setSettingsOpen((value) => !value)}
          >
            <SettingsIcon size={18} />
          </button>
        </div>
      </header>

      <section className="search-band">
        <div className="search-box">
          <Search size={20} aria-hidden="true" />
          <input
            ref={searchRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Szukaj tekstu, adresu URL albo domeny"
            aria-label="Szukaj w historii schowka"
          />
        </div>
        {message && <span className="toast">{message}</span>}
      </section>

      {settingsOpen && (
        <section className="settings-panel" aria-label="Ustawienia historii">
          <label>
            Retencja
            <select
              value={retentionValue}
              onChange={(event) =>
                setSettings((value) => ({
                  ...value,
                  retentionDays:
                    event.target.value === 'none' ? null : Number(event.target.value),
                }))
              }
            >
              <option value="1">1 dzień</option>
              <option value="7">7 dni</option>
              <option value="30">30 dni</option>
              <option value="none">Bez limitu czasu</option>
            </select>
          </label>
          <label>
            Limit wpisów
            <input
              type="number"
              min={25}
              max={10000}
              step={25}
              value={settings.maxItems}
              onChange={(event) =>
                setSettings((value) => ({ ...value, maxItems: Number(event.target.value) }))
              }
            />
          </label>
          <label>
            Globalny skrót
            <input
              value={settings.shortcut}
              onChange={(event) =>
                setSettings((value) => ({ ...value, shortcut: event.target.value }))
              }
            />
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={settings.hideAfterCopy}
              onChange={(event) =>
                setSettings((value) => ({ ...value, hideAfterCopy: event.target.checked }))
              }
            />
            Ukryj po ponownym skopiowaniu
          </label>
          <div className="settings-actions">
            <button type="button" className="secondary-button" onClick={clearHistory}>
              <Eraser size={16} />
              Wyczyść historię
            </button>
            <button type="button" className="primary-button" onClick={saveSettings}>
              Zapisz
            </button>
          </div>
        </section>
      )}

      <section className="results" aria-label="Historia schowka" ref={resultsRef}>
        {items.length === 0 ? (
          <div className="empty-state">
            <Clipboard size={36} />
            <p>Skopiuj tekst, link albo obraz, a wpis pojawi się tutaj.</p>
          </div>
        ) : (
          items.map((item, index) => (
            <article
              className={index === selected ? 'history-row selected' : 'history-row'}
              key={item.id}
              onClick={() => copyItem(item)}
            >
              <div className="type-mark">{iconForKind(item.kind)}</div>
              {item.kind === 'image' ? (
                <img className="thumb" src={item.thumbDataUrl ?? ''} alt="Miniatura schowka" />
              ) : (
                <div className="content">
                  <div className="title-line">
                    <span>{item.kind === 'link' ? item.domain ?? item.url : item.content}</span>
                  </div>
                  <ItemSource item={item} />
                </div>
              )}
              {item.kind === 'image' && (
                <div className="content">
                  <div className="title-line">
                    <span>{item.content}</span>
                  </div>
                  <p>{item.sourceDomain ?? item.sourceApp ?? 'Obraz ze schowka'}</p>
                </div>
              )}
              <div className="meta">
                <span>{kindLabel[item.kind]}</span>
                <span>{formatTime(item.createdAt)}</span>
                {(item.sourceDomain || item.domain) && (
                  <span className="domain">
                    <Globe size={13} />
                    {item.sourceDomain ?? item.domain}
                  </span>
                )}
              </div>
              {item.sourceUrl ? (
                <button
                  className="icon-button source"
                  type="button"
                  title={`Otwórz dokładny link źródłowy: ${item.sourceUrl}`}
                  onClick={(event) => {
                    event.stopPropagation()
                    openSourceUrl(item)
                  }}
                >
                  <ExternalLink size={16} />
                </button>
              ) : (
                <span className="icon-placeholder" aria-hidden="true" />
              )}
              <button
                className="icon-button quiet"
                type="button"
                title="Skopiuj ten wpis"
                onClick={(event) => {
                  event.stopPropagation()
                  copyItem(item)
                }}
              >
                <Copy size={16} />
              </button>
              <button
                className="icon-button danger"
                type="button"
                title="Usuń wpis"
                onClick={(event) => {
                  event.stopPropagation()
                  deleteItem(item)
                }}
              >
                <Trash2 size={16} />
              </button>
            </article>
          ))
        )}
      </section>
    </main>
  )
}

function iconForKind(kind: ClipboardKind) {
  if (kind === 'link') return <Link size={18} />
  if (kind === 'image') return <ImageIcon size={18} />
  return <Type size={18} />
}

function ItemSource({ item }: { item: ClipboardItem }) {
  if (item.kind === 'link') {
    return <p className="item-url">{item.url}</p>
  }

  if (item.sourceTitle) {
    return <p>{item.sourceTitle}</p>
  }

  return <p>{item.sourceApp ?? ''}</p>
}

function formatTime(value: number) {
  return new Intl.DateTimeFormat('pl-PL', {
    day: '2-digit',
    month: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value * 1000))
}

export default App
