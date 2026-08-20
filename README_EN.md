# Missevan FM Recorder

**An automated acquisition and archival system for Missevan FM (猫耳 FM) live audio streams**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/blob/main/LICENSE)
[![Version: 0.2.0](https://img.shields.io/badge/version-0.2.0-blue.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/thestmitsuki/Missevan-FM-Recorder/ci.yml?branch=main&label=CI)](https://github.com/thestmitsuki/Missevan-FM-Recorder/actions)
[![Platform: Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)](https://github.com/thestmitsuki/Missevan-FM-Recorder)

[中文](README.md) | **English**

> ⚠️ **Disclaimer**: This software is provided for personal learning and research purposes only. Users are responsible for complying with the Missevan FM terms of service and applicable laws. Copyright of recorded content belongs to its respective owners. The developer assumes no liability arising from the use of this tool.

## Table of Contents

- [1. Project Overview](#1-project-overview)
- [2. Operating Principle](#2-operating-principle)
- [3. Functional Specifications](#3-functional-specifications)
- [4. System Requirements](#4-system-requirements)
- [5. Installation & Deployment](#5-installation--deployment)
- [6. Usage Guide](#6-usage-guide)
- [7. Engineering & Development](#7-engineering--development)
- [8. Contributing](#8-contributing)
- [9. License](#9-license)

## 1. Project Overview

Missevan FM Recorder is a desktop application for Windows 10 / 11 (built with Tauri 2; Rust backend, Vue 3 frontend) that automates the acquisition, archival, and management of **audio streams** from streamers' live rooms on the Missevan FM platform.

The system is designed to run unattended: it continuously monitors the live status of followed streamers, triggers audio recording when a stream starts, and provides structured organization and retrieval of the recorded artifacts. Only the audio track is captured — no video or danmaku content is involved.

**Technology stack**:

| Layer | Technology |
| --- | --- |
| Desktop framework | Tauri 2 (dual-window: main window + setup wizard) |
| Backend | Rust (2021 edition) · tokio async runtime |
| Frontend | Vue 3.5 · TypeScript (strict) · Vite 8 · Pinia 4 · Tailwind CSS 4 |
| i18n | vue-i18n 10 (Simplified Chinese / English) |
| Recording engine | FFmpeg / ffprobe (child-process invocation) |

## 2. Operating Principle

The system follows a main pipeline of "**polling detection → recording execution → file archival → event notification**":

```
Periodic live-status polling → live-state determination → disk precheck
→ stream URL acquisition → delay-window recheck → FFmpeg child process
→ segment-based audio write → process monitoring (5 s interval)
→ end-of-recording cleanup → file-cache refresh → event notification
```

### 2.1 Live Detection

- The detection loop polls at a configurable interval (default `check_interval_secs = 120`), with an additional 0–60 s random jitter per cycle to reduce periodic request signatures against the platform API.
- Live-state determination uses **double verification**: the platform API status and the recording-side state are merged (`merge_live_state`) into a final live status, avoiding misjudgment from a single data source.
- Request errors are handled by category: server errors (5XX / HTTP 429) trigger retry with exponential backoff, with additional cooldown on 429; network errors trigger retry; format and unknown errors do not retry.

### 2.2 Recording Execution

- Once a live stream is confirmed, the system sequentially: prechecks disk space (threshold `disk_space_limit_gb`) → resolves the stream URL → enters a cancellable delay window (`pre_record_delay_secs`) → rechecks that the stream is still live and not already recording, then launches the FFmpeg child process.
- FFmpeg captures the audio track only (`-vn`), supporting M4A / MP3 containers, segment-based recording (`-f segment`), and bitrates of 64 / 128 / 192 / 256 / 320 kbps (default 128).
- Concurrency protection: per-streamer deduplication, an active-task cap, and a process-level single-instance file lock form three layers of defense against duplicate recordings.
- On abnormal child-process exit, the system retries with exponential backoff (crash circuit breaker) and retains `.part` residue markers for troubleshooting.

### 2.3 File Management & Archival

- The output directory is maintained by a file-cache scanning service. The frontend groups files by date (Today / Yesterday / This Week / This Month / Year-Month); segments auto-collapse into groups and support continuous playback.
- Search, date filtering, rename, and delete are supported; **files being recorded are protected** from rename and deletion.
- End-of-recording cleanup deletes expired files by retention days (`retention_days`) or a total-size cap (`max_total_gb`).

### 2.4 Notifications & Resident Operation

- A notification dispatcher routes events through a filter matrix (master switch + 7 event categories): native Windows toast (registered via app AUMID), system sound, and an in-app notification center (ring buffer of 500 entries).
- The system tray stays resident with a dynamic menu showing recording status and recent recordings (up to 5); closing the main window hides it to the tray by default, and exit runs a unified shutdown flow.
- A first-run wizard performs environment checks (FFmpeg / ffprobe candidates, disk space, write permissions) and can automatically download the portable FFmpeg build on Windows.
- Update checks parse the `v{version}` tag from the GitHub Releases API and compare it against the current version.

## 3. Functional Specifications

| Domain | Specification |
| --- | --- |
| Live monitoring | Configurable interval (default 120 s + 0–60 s jitter); double-verified live determination; exponential backoff on 429 |
| Auto recording | Audio track only; M4A / MP3; 64–320 kbps (default 128); segment recording; configurable concurrency cap |
| File management | Date grouping; segment collapsing; continuous playback; search / filter / rename / delete; protection for active recordings; auto cleanup |
| Streamer tags | 5 fixed categories: Music / Singing / Daily / ASMR / Chat |
| Notifications | Native Windows toast + sound; configurable event types and sounds |
| Background operation | System tray; optional start on boot; single-instance enforcement |
| Bilingual UI | Simplified Chinese / English; light / dark / system themes |
| Debug panel | Live logs / network records / engine state / Mock environment (off by default); sanitized diagnostic report export |
| Data safety | Atomic config writes with automatic backups (5 retained); sensitive fields obfuscated on export (enc:v1:) |

## 4. System Requirements

| Item | Requirement |
| --- | --- |
| OS | Windows 10 / 11 (Linux is compilable; official installers are Windows NSIS only) |
| Runtime | WebView2 Runtime (built into Windows 11; required on Windows 10) |
| Recording engine | FFmpeg / ffprobe (fetched by the first-run wizard, or placed in the `ffmpeg/` folder next to the app) |

## 5. Installation & Deployment

1. Download the latest installer from the [Releases](https://github.com/thestmitsuki/Missevan-FM-Recorder/releases) page.
2. Run the installer; on first launch the setup wizard walks through the basic configuration (output directory, recording format, etc.).
3. After the wizard completes environment checks and FFmpeg readiness validation, the main interface opens and the detection loop starts.

## 6. Usage Guide

### 6.1 Adding Streamers

On the **Live** page, click **＋** and enter a room URL (format: `https://fm.missevan.com/live/{numeric room id}`). The system automatically resolves the room ID, streamer name, and avatar.

> Some rooms require an authenticated session to record; a per-streamer Cookie can be configured in the streamer settings.

### 6.2 Configuration

| Domain | Entry | Key parameters |
| --- | --- | --- |
| Recording | Settings → Recording | Output directory, container format, bitrate, segment policy, concurrency cap, auto cleanup |
| Notifications | Settings → Notifications | Event types, sounds |
| General | Settings → General | Language, theme, start on boot |
| Diagnostics | Settings → About | Debug panel, diagnostic report export |

### 6.3 Data Locations

- Recordings: output directory, organized by **streamer name / date** (default filename template: `{streamer}/{room_id}/{date}/{time}.{ext}`).
- Configuration and logs: `%APPDATA%\missevan-recorder\` (logs under the `logs/` subdirectory).

## 7. Engineering & Development

> For architecture details, command/event lists, and the test matrix, see the [`DOCS/`](DOCS/README.md) collection (in Chinese).

**Code organization** (three-layer backend with unidirectional dependencies):

| Layer | Responsibility |
| --- | --- |
| `api` | Thin Tauri command layer: parameter validation → domain calls → error wrapping (9 modules, 54 commands) |
| `domain` | Business rules (config / detection / recording / file services / platform client / tools), pure Rust, unit-testable |
| `infrastructure` | Platform adaptation (state / logging / notifications / tray / health checks / single-instance lock) |

**Build & verification**:

```bash
npm ci                     # Install dependencies
npm run build              # Frontend type-check (vue-tsc) + build
npm run tauri dev          # Dev mode (hot reload)
npm run tauri build        # Package the installer
cd src-tauri && cargo test # Backend unit tests
```

CI (`.github/workflows/ci.yml`) runs on every push / PR: frontend `vue-tsc` type-check + `vite build`; backend `cargo check` + `cargo test`.

## 8. Contributing

Issues and pull requests are welcome. For bug reports, please use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.yml) and include the app version, OS, reproduction steps, and logs (`%APPDATA%\missevan-recorder\logs\`).

## 9. License

[MIT](LICENSE) © thestmitsuki

The bundled FFmpeg binaries are subject to their own LGPL/GPL licenses.
