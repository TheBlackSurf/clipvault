# ClipVault

ClipVault is a lightweight local clipboard history app for macOS built with Tauri, React, Rust, and SQLite.

It runs in the background, saves copied text, links, and screenshots, and lets you bring back older clipboard entries with a global shortcut.

## Features

- Local clipboard history for text, URLs, and images
- Fast search across copied text, URLs, and domains
- Image thumbnails for screenshots and copied images
- Global shortcut: `Cmd+Shift+S` on macOS
- Keyboard-first flow: search, arrow-key navigation, `Enter` to copy
- Automatically hides after copying and returns focus to the previous app
- Tray/menu bar controls
- Retention settings: 1 day, 7 days, 30 days, or no time limit
- Maximum item limit
- Pause monitoring
- Delete one item or clear all history
- Local SQLite storage, no cloud backend

## Privacy

Clipboard history can contain sensitive data. ClipVault stores data only on your computer in the app data directory.

The app tries to avoid saving clipboard content from password managers and password-related windows, but this is best-effort. Use pause mode before copying secrets you do not want stored.

On macOS, ClipVault can ask for Automation or Accessibility permissions so it can detect the active browser/app, read the active tab URL when available, register the global shortcut, and restore focus after choosing an item.

## Source Links

When possible, ClipVault stores the source page for copied content:

- Exact link from clipboard HTML when the browser/page provides it
- Active browser tab URL as a fallback
- Active app/window title when URL access is unavailable

Browser and website behavior varies. For perfect source tracking, a browser extension would be the next step.

## Install on macOS

Download the latest `.dmg` from the GitHub Releases page, open it, and drag ClipVault into Applications.

The current macOS build is unsigned. On first launch, macOS may require opening it through:

1. System Settings
2. Privacy & Security
3. Open Anyway

## Development

Requirements:

- Node.js
- Rust
- npm

Install dependencies:

```bash
npm install
```

Run in development:

```bash
npm run tauri:dev
```

Build the macOS app and installer:

```bash
npm run tauri:build
```

Generated macOS artifacts are written under:

```bash
src-tauri/target/release/bundle/
```

## Checks

```bash
npm run lint
npm run build
cd src-tauri && cargo test && cargo check
```

## Tech Stack

- Tauri 2
- Rust
- React
- TypeScript
- SQLite with FTS5
- Vite

## License

MIT
