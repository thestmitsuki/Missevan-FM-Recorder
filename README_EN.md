# Missevan FM Recorder

**Automatic live-stream audio recorder for Missevan FM (猫耳 FM)** — detects when your followed streamers go live, records audio hands-free, and manages the files for you. Runs unattended in the background.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/blob/main/LICENSE)
[![Version: 0.1.0](https://img.shields.io/badge/version-0.1.0-blue.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/thestmitsuki/Missevan-FM-Recorder/ci.yml?branch=main&label=CI)](https://github.com/thestmitsuki/Missevan-FM-Recorder/actions)
[![Platform: Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder)

[中文](README.md) | **English**

> ⚠️ **Disclaimer**: This software is provided for personal learning and research purposes only. Please comply with the Missevan FM terms of service and applicable laws. Copyright of recorded content belongs to its respective owners. The developer assumes no liability arising from the use of this tool.

## Features

- 🎙️ **Live monitoring** — polls your followed streamers for "on air" status. The polling interval and random jitter are configurable (default 120 s + 0–60 s jitter to reduce platform risk-control flags). "On air" is confirmed by double verification (API status + recording state), with automatic exponential backoff on HTTP 429 rate limits
- 📹 **Automatic recording** — kicks off FFmpeg automatically when a stream goes live (audio track only). Supports M4A / MP3, segment-based recording, and bitrates of 64/128/192/256/320 kbps (default 128). The maximum number of concurrent recordings is configurable
- 📁 **File management** — recordings are grouped by date (Today / Yesterday / This Week / This Month / Year-Month), with segments auto-collapsed into groups; search, date-range filtering, a built-in player with continuous playback (plays whole segment groups in sequence), plus rename and delete (files being recorded are protected from both)
- 🏷️ **Streamer tags** — 5 fixed tags (Music / Singing / Daily / ASMR / Chat) for categorization and filtering
- 🔔 **System notifications** — native Windows toast notifications sent under the app's own identity (AUMID registered by the app itself — no PowerShell fallback), with the system default sound; event types and sound can be configured
- 🖥️ **System tray** — minimizes to the tray and keeps running in the background; the tray menu shows live recording status and recent files (up to 5), clickable to open
- 🌐 **Bilingual UI** — full i18n (Simplified Chinese / English), light / dark / system theme, and adjustable accent color
- 🧪 **Debug panel** — live logs, network request tracing, detector/recorder engine status, and a mock live-streaming environment (off by default; enable it under **Settings → About**), with one-click export of a sanitized diagnostic report

**Also worth mentioning**: first-run setup wizard (environment checks for FFmpeg / ffprobe / disk space / write permission, with automatic download of a portable FFmpeg build) · per-streamer Cookie support (for streams that require authentication) · automatic cleanup (retention days / total size cap / scheduled time) · automatic update checks · optional start-on-boot · single-instance enforcement.

## Quick Start

1. **Download** — grab the latest NSIS installer from the [Releases](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases) page. Requires Windows 10 / 11 (WebView2 Runtime needed; built into Windows 11)
2. **Install & configure** — run the installer; the first-run wizard walks you through the output directory, recording format, and other basics, and checks / downloads FFmpeg automatically (you can also drop it into the `ffmpeg/` folder next to the executable)
3. **Add streamers** — click **+** on the Live page and paste a room URL (e.g. `https://fm.missevan.com/live/100000001`) to start monitoring

## Usage

### Getting a room URL

- Open the stream in the Missevan FM mobile app or web client and copy the address bar link; the format is `https://fm.missevan.com/live/<number>`
- Paste the URL when adding a streamer in the app — the room ID is extracted automatically and the streamer's name and avatar are fetched. If a stream requires a signed-in session, fill in a Cookie in the streamer settings

### Streamers and tags

- Toggle automatic monitoring per streamer, set an alias and tags
- Tags power the category filter on the Files page; each streamer can have multiple tags

### Key settings

- **Recording** (Settings → Recording): output directory, format (M4A / MP3), bitrate, segment length, filename template, max concurrent recordings
- **Notifications** (Settings → Notifications): master switch, event types, sound on/off
- **General** (Settings → General): language, theme, start on boot, close behavior (minimize to tray / exit)
- **Debug panel** (Settings → About): enabling it adds a "Debug Panel" entry to the main navigation
- **Logs**: stored in `%APPDATA%\missevan-recorder\logs\`, rotated daily

## FAQ

- **Why isn't going-live detected?** Check that automatic monitoring is enabled for that streamer. The default polling interval is 120 s, so a newly started stream is picked up within one interval (plus random jitter) at most
- **The recording is silent / the file is empty?** Make sure FFmpeg is installed correctly (Settings → Advanced, or the wizard's environment check). Some streams may require a Cookie (set it in the streamer settings)
- **FFmpeg download failed?** The wizard offers a manual download link; alternatively, get FFmpeg from [ffmpeg.org](https://ffmpeg.org/download.html) and put `ffmpeg.exe` / `ffprobe.exe` in the `ffmpeg/` folder next to the executable, or point the app at them in settings
- **No notification sound?** Check Windows "Focus Assist / Do Not Disturb" settings; notification sounds follow the system default (and can be turned off in notification settings)
- **Shows "on air" but nothing was recorded?** Going-live is double-verified (API + recording state); brief discrepancies during API flakiness are normal and self-correct on the next polling round
- **Why are recordings stored in a folder named after the streamer, with the streamer's name in the filename?** The default filename template is `{anchor_name}/{date}_{time}_{anchor_name}_{index}.{ext}` — the streamer's name is a stable identifier (live titles change, so they are not used in filenames). Files go into a per-streamer folder with date/time and a per-streamer sequence number in the name. You can customize the template under Settings → Recording; supported placeholders: `{anchor_name}` (streamer name), `{room_id}` (room ID), `{date}` (date), `{time}` (time), `{index}` (per-streamer recording sequence), `{ext}` (format extension)
- **Where are the logs? How do I report a bug?** Logs live in `%APPDATA%\missevan-recorder\logs\` (rotated daily). You can also export a full sanitized diagnostic report from the Debug Panel — attach it when reporting issues
- **Why doesn't my setting take effect?** Start-on-boot, close behavior, tray visibility, and log level take effect after restarting the app (marked "restart required" in the UI). "Custom DNS" is a UI placeholder not yet wired into the runtime (marked "not effective")

## Known Limitations

- "Custom DNS" (Settings → Network) is marked "not effective" — it is a UI placeholder with no runtime wiring yet
- Global shortcuts are display-only placeholders; the backend registration is not implemented and editing is disabled (planned for a future release)
- The performance monitor in the Debug Panel is an experimental placeholder
- Danmaku (bullet-comment) recording is not planned — this tool records audio only
- Some settings (start-on-boot / close behavior / tray visibility / log level) only take effect after restarting the app

## Development

### Tech Stack

- Frontend: Vue 3 · Vite · TypeScript · Pinia · vue-i18n · Tailwind CSS v4 · shadcn-vue (local components)
- Backend: Rust · Tauri 2 · tokio · reqwest

### Build

```bash
# Frontend (install deps + type-check + build)
npm ci
npx vue-tsc -p tsconfig.app.json --noEmit
npm run build

# Backend (Windows)
cd src-tauri
cargo check
cargo test
cd ..

# Dev run / package the installer
npm run tauri dev
npm run tauri build
```

### Project Layout

```
src/              Frontend (views / stores / services / components / locales)
src-tauri/src/    Backend (api Tauri commands / domain logic / infrastructure)
src-tauri/icons/  App icons
.github/          CI workflows (frontend type-check + build, backend cargo check + test) and issue templates
```

## Contributing

Issues and pull requests are welcome:

1. **Report a bug** — file an issue via [GitHub Issues](https://github.com/thestmitsuki/Missevan-FM-Recorder/issues) using the [bug report template](https://github.com/thestmitsuki/Missevan-FM-Recorder/issues/new?template=bug_report.yml). Please include: app version (Settings → About), OS and architecture, reproduction steps, and the app logs (`%APPDATA%\missevan-recorder\logs\`) or a diagnostic report exported from the Debug Panel
2. **Submit code** — fork the repository, make your changes on a separate branch, and open a pull request. CI automatically runs the frontend type-check and build plus the backend `cargo check` and tests — make sure everything passes before requesting a merge
3. **Code style** — match the existing conventions (strict TypeScript on the frontend; rustfmt defaults and the api / domain / infrastructure layering on the backend)

## License

[MIT](LICENSE) © thestmitsuki

The bundled FFmpeg binaries are subject to their own LGPL/GPL licenses.
