# Isle — a Dynamic Island for Windows 11

A glanceable island that floats at the top-center of your primary display: now-playing,
timers, weather, system stats, clipboard history, notification mirroring, Bluetooth
audio events, synced lyrics, and TV/movie info — in one compact surface that gets out
of the way.

Local-first by design: no accounts, no telemetry, no analytics. Sensitive features
(clipboard history, notification mirroring, the phone bridge) are **off by default**
and stay on your machine.

![stack](https://img.shields.io/badge/Tauri%202-Rust%20%2B%20React-blue)

## Features

| Area | What you get | Data source |
| --- | --- | --- |
| Media | Album art, transport controls, now-playing pill | Windows SMTC (GlobalSystemMediaTransportControls) — works with Spotify, browsers, mpv, etc. |
| Timers | Multiple named countdowns, presets, pause/resume, completion alert | Local (Rust) |
| Weather | Current conditions, hi/lo, wind, rain chance, city search | Open-Meteo (keyless) |
| System | CPU / RAM bars, network throughput, mini history sparkline | sysinfo |
| Lyrics | Line-synced karaoke view with full-lyrics mode | LRCLIB (keyless) |
| TV / movies | Poster + episode summaries for what you're watching | TVmaze (keyless); TMDB optional |
| Bluetooth | Connect/disconnect animations for audio endpoints | Windows.Devices.Enumeration |
| Clipboard | Opt-in history with search, masking, copy/delete | Local, in-memory |
| Notifications | Opt-in toast mirroring with a feed + clear | UI Automation |
| Phone | LAN notification relay from your phone | Token-gated HTTP bridge |

**Honest limits:** SMTC in the windows-rs projection exposes no timeline-write API, so
there is no seek scrubber (play/pause/skip work). Notification mirroring reads banner
text while toasts are visible on screen — it cannot see notifications delivered during
Focus Assist. The phone bridge relays and dismisses but never promises replies (a
device-side agent would be required; see below).

## Setup

Prerequisites: Windows 11, WebView2 Runtime (preinstalled on updated Windows 11).

```powershell
# Development
npm install
npm run dev        # vite + cargo dev build, island appears after page load

# Production build (exe + NSIS installer in src-tauri/target/release/bundle)
npm run build
```

The exe lives at `src-tauri/target/release/isle.exe`. Single-instance: launching a
second copy pokes the running one to show itself instead of starting a duplicate.

### First run

- The island appears top-center as a compact pill (clock by default). Hover to expand
  (or click, per settings). Press `Esc` to collapse.
- The tray icon has **Settings**, **Show/Hide Island**, and **Quit**.
- Settings → *Privacy* toggles clipboard history and notification mirroring (both off
  by default). Settings → *Media & weather* picks your city and optional TMDB key.

## Permissions and data handling

- **Media (SMTC):** reads the system media session; no permission prompt needed.
- **Clipboard history (opt-in):** text copies are kept **in memory only** (bounded by
  *Clipboard retention*, default 100, max 500). Entries that look like payment cards
  (Luhn-checked), IBANs, or long token-like strings are **masked** in the UI; reveal
  and copy are explicit per-entry actions. Nothing is written to disk; nothing leaves
  the machine; *Disable* wipes the list.
- **Notification mirroring (opt-in):** a UI-Automation hook reads text from Windows
  toast banner windows (class `Windows.UI.Core.CoreWindow`, title *New notification*)
  as they appear. This works for unpackaged apps, where `UserNotificationListener`
  does not (it requires package identity). The feed holds the last 50 items in memory;
  *Clear* wipes it. Toast re-renders during their entrance animation are deduped.
- **TMDB API key:** stored in **Windows Credential Manager** (DPAPI-backed) under
  `Isle:tmdb` — never in the settings file, never logged.
- **Settings file:** `%APPDATA%\com.stanhardy.isle\settings.json`.
- **Network:** requests go only to Open-Meteo, LRCLIB, TVmaze, TMDB/iTunes when those
  features are enabled. No other endpoints, no telemetry.

## Phone companion

Enable *Settings → Phone companion* to start the bridge on `0.0.0.0:7350` (Windows
Firewall will prompt once). Pairing is a shared token shown in settings — regenerate
it anytime.

Send a notification from any phone automation app (Tasker, HTTP Shortcuts,
MacroDroid, …):

```
POST http://<pc-ip>:7350/notify
Content-Type: application/json

{"token": "<token>", "app": "WhatsApp", "title": "Morgan", "body": "ping"}
```

- `GET /ping` → `{"status":"ok","app":"isle"}`
- `POST /action` with `{"token":"…","action":"dismiss","id":…}` (or no `id` to clear)
- Replying from the PC returns **501** by design: a remote HTTP caller cannot post a
  phone-side reply without a device-side agent, and this bridge does not pretend to.
- The token is required on every mutating endpoint; the endpoint only listens on your
  LAN.

## Customization

- **Modules:** reorder/toggle every card (media, timer, weather, metrics, info,
  lyrics, bluetooth, notifications, clipboard) in Settings → Modules.
- **Behavior:** hover-vs-click expansion, island scale (0.6–1.5×), theme
  (system/dark/light), hide-in-fullscreen.
- **Units:** metric or imperial.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| Island never appears | Another Isle instance holds the single-instance lock — check the tray and `Get-Process isle`. |
| No media info | The app you're playing from must publish an SMTC session (Chrome/Spotify/mpv do; some games don't). |
| Album art missing after a track change | Art is re-sent only when title/artist/album/duration change; some players send artless metadata — the island keeps the last art for the same track. |
| Weather shows an error | Open-Meteo outage or no city selected — Settings → Media & weather → search a city. |
| "No lyrics found" | LRCLIB doesn't cover every track; obscure/short-name tracks often miss. |
| Phone bridge unreachable | Firewall prompt was denied, or the phone is on a guest/isolated Wi-Fi. Re-enable to re-prompt. |
| Island hides during fullscreen | Intentional (Settings → Island). The watcher has a 6 s launch grace to avoid false positives. |
| Settings keys reset | `settings.json` merges over defaults on load; corrupt files fall back to defaults. |

## Architecture

```
src/               React UI — island pill/panel (src/island), settings (src/settings)
src/shared/        typed invoke/event bindings, spring engine
src-tauri/src/
  media.rs         SMTC session watcher (blocking WinRT on spawn_blocking threads)
  timers.rs        countdown registry + tick emitter
  metrics.rs       sysinfo sampler
  clipboard.rs     clipboard sequence poller, sensitivity masking, copy-back
  notify.rs        WinEvent hook + UIA toast mirror
  bluetooth.rs     DeviceWatcher on KSCATEGORY_RENDER endpoints
  fullscreen.rs    foreground-window fullscreen probe (hides island)
  island_window.rs overlay placement, WS_EX_NOACTIVATE, topmost reassert
  config.rs        settings load/merge/save (atomic write)
  credentials.rs   Windows Credential Manager (TMDB key)
  providers/       open_meteo, lrclib, tmdb (TVmaze + TMDB + iTunes)
  bridge.rs        token-gated LAN HTTP (tiny_http)
```

Windows-rs COM interfaces are not `Send`, so every WinRT interaction runs on blocking
worker threads (`tauri::async_runtime::spawn_blocking`) with MTA COM initialized; the
SMTC manager is re-acquired per work item (it is a per-process singleton).

## Tests

```powershell
cargo test --manifest-path src-tauri/Cargo.toml   # 21 unit tests (pure logic)
npm run test:fe                                   # vitest: spring engine + island render
```

## License

MIT
