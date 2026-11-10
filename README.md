# Isle — a Dynamic Island for Windows 11

A glanceable island that lives at the top of your primary display: now-playing,
timers, weather, system stats, clipboard history, notification mirroring,
Bluetooth audio events, synced lyrics, and TV/movie info — in one compact
surface that gets out of the way.

Local-first by design: no accounts, no telemetry, no analytics. Sensitive
features (clipboard history, notification mirroring, the phone companion
bridge) are **off by default** and stay on your machine.

Isle is the Tauri 2 + Rust + React rewrite of the original Electron Isle.

## Features

| Area | What you get | Data source |
| --- | --- | --- |
| Media | Album art, transport controls, spectrum bars, now-playing pill | Windows SMTC (GlobalSystemMediaTransportControls) |
| Timers | Multiple named countdowns, presets, pause/resume, completion alert | Local (Rust) |
| Weather | Current conditions, hi/lo, wind, rain chance, city search | Open-Meteo (keyless) |
| System | CPU / RAM bars, network throughput, mono telemetry numerals | sysinfo |
| Lyrics | Line-synced karaoke view with full-lyrics mode | LRCLIB (keyless) |
| TV / movies | Poster + episode summaries for what you're watching | TVmaze (keyless); TMDB optional |
| Bluetooth | Connect/disconnect animations for audio endpoints | Windows.Devices.Enumeration |
| Clipboard | Opt-in history with search, sensitive-entry masking, copy/delete | Local, in-memory |
| Notifications | Opt-in toast mirroring with a feed + clear | UI Automation |
| Phone | LAN notification relay from your phone | Token-gated HTTP bridge |
| Updates | Signed in-app updater (check + install from GitHub releases) | tauri-plugin-updater |

**Honest limits:** Windows SMTC exposes no remote seek-to-position API, so the
progress bar is display-only (play/pause/skip work). The notification mirror
reads banner text while toasts are on screen — it cannot see notifications
delivered during Focus Assist. The phone bridge relays and dismisses but never
promises replies (a device-side agent would be required). The UIA-based mirror
matches the localized "New notification" banner title for 28 languages.

## Island designs

Settings → Island → **Island design** switches the look live:

- **Pill** — floating glass capsule, flush with the top edge (default)
- **Notch** — opaque black, anchored to the screen edge, hangs down
- **Black** — Droppy-style opaque capsule
- **Clear** — Droppy-style transparent glass
- **Strip** — Peeka-style full-width top bar (panel drops down beneath it)

The spectrum bars have their own color themes: accent, white, gradient,
album tint, or mono.

## Setup

Prerequisites: Windows 11, WebView2 Runtime (preinstalled on updated Windows 11).

**Install:** grab `Isle_1.0.0_x64-setup.exe` from the
[releases page](https://github.com/StanHardy/isle/releases/latest) — it
installs per-user (`%LOCALAPPDATA%\Isle`), adds a Start Menu shortcut, and the
app checks GitHub releases for updates (Settings → About → Check for updates).

**Development:**

```powershell
npm install
npm run dev        # vite + cargo dev build
npm run build      # signed release build (requires updater signing keys)
npm test           # 29 Rust tests + 8 frontend tests
```

Release builds are signed with a minisign key (`tauri signer generate`); the
public key ships in `src-tauri/tauri.conf.json` and the private key stays out
of the repo (`TAURI_SIGNING_PRIVATE_KEY` env at build time).

### First run

- The island appears at the top-center of your primary display as a compact
  pill. Hover to expand (or click, per settings). Press `Esc` to collapse.
- The tray icon has **Settings**, **Show/Hide Island**, and **Quit**.
- Settings → *Privacy* toggles clipboard history and notification mirroring
  (both off by default).

## Privacy and data handling

- **Media (SMTC):** reads the system media session; no permission prompt needed.
- **Clipboard history (opt-in):** text copies are kept **in memory only**
  (bounded list, default 100 entries, max 500). Entries that look like payment
  cards (Luhn-checked), IBANs, or long token-like strings are **masked** in the
  UI; reveal and copy are explicit per-entry actions. Nothing is written to
  disk; nothing leaves the machine; *Disable* wipes the list.
- **Notification mirroring (opt-in):** a UI-Automation hook reads text from
  Windows toast banner windows as they appear. The feed holds the last 50 items
  in memory; *Clear* wipes it.
- **TMDB API key:** stored in **Windows Credential Manager** (DPAPI-backed)
  under `Isle:tmdb` — never in the settings file, never logged.
- **Settings file:** `%APPDATA%\com.stanhardy.isle\settings.json`.
- **Diagnostics:** `%APPDATA%\com.stanhardy.isle\debug.log` (rotated at 256 KB)
  — contains service errors and playback metadata, never clipboard contents.
- **Network:** requests go only to Open-Meteo, LRCLIB, TVmaze, TMDB/iTunes when
  those features are enabled, and to GitHub for update checks. Image fetches
  are domain-allowlisted. No telemetry, no analytics, no accounts.

## Phone companion

Enable *Settings → Phone companion* to start the bridge on `0.0.0.0:7350`
(Windows Firewall prompts once). Pairing is a shared token shown in settings —
regenerate it anytime.

Send a notification from any phone automation app (Tasker, HTTP Shortcuts,
MacroDroid, …):

```
POST http://<pc-ip>:7350/notify
Content-Type: application/json

{"token": "<token>", "app": "WhatsApp", "title": "Morgan", "body": "ping"}
```

- `GET /ping` → `{"status":"ok","app":"isle"}`
- `POST /action` with `{"token":"…","action":"dismiss","id":…}` (or no `id` to clear)
- Per-IP rate limiting (20 requests / 10 s)
- Replying from the PC returns **501** by design: a remote HTTP caller cannot
  post a phone-side reply without a device-side agent, and this bridge does
  not pretend to.

## Island designs

Settings → Island → *Island design*:

- **Pill** (default) — floating glass capsule, flush with the screen edge
- **Notch** — opaque black bar anchored to the screen edge, rounded bottom
  corners, hangs down (window top is pushed above the screen)
- **Black** — opaque capsule, no blur
- **Clear** — transparent glass, heavy blur
- **Strip** — full-width 48 px top bar; the panel drops down beneath it and
  expansion is click-driven from the strip's center zone

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| Island never appears | Another Isle instance holds the single-instance lock — check the tray and `Get-Process isle`. |
| No media info | The app you're playing from must publish an SMTC session (Chrome/Spotify/mpv/mpc-hc do; some games don't). |
| Album art missing after a track change | Art is re-sent only when title/artist/album/duration change, with 3 retry attempts per track; players without thumbnails fall back to the TV/movie poster when one is found. |
| Weather shows an error | Open-Meteo outage or no city selected — Settings → Media & weather → search a city. |
| "No lyrics found" | LRCLIB doesn't cover every track; obscure/short-name tracks often miss. |
| Phone bridge unreachable | Firewall prompt was denied, or the phone is on a guest/isolated Wi-Fi. Re-enable to re-prompt. |
| Island hides during fullscreen | Intentional (Settings → Island). The watcher has a 6 s launch grace to avoid false positives. |
| Update check fails | The GitHub release feed may not exist yet for a fresh repo, or you're offline. |
| Settings keys reset | `settings.json` merges over defaults on load; invalid update payloads are rejected (nothing is silently reset). |

## Architecture

```
src/               React UI — island pill/panel/strip (src/island), settings (src/settings)
src/shared/        typed invoke/event bindings, spring engine, spectrum bars
src-tauri/src/
  media.rs         SMTC session watcher + artwork (blocking WinRT workers)
  spectrum.rs      WASAPI loopback capture + 2048-pt FFT, AGC-normalized bands
  timers.rs        countdown registry + tick emitter
  metrics.rs       sysinfo sampler
  clipboard.rs     clipboard sequence poller, sensitivity masking, copy-back
  notify.rs        WinEvent hook + UIA toast mirror (29 locales)
  bluetooth.rs     DeviceWatcher on KSCATEGORY_RENDER endpoints
  fullscreen.rs    foreground-window fullscreen probe (hides island)
  island_window.rs placement, designs, rounded region, no-focus-steal
  config.rs        settings load/merge/save (atomic write)
  credentials.rs   Windows Credential Manager (TMDB key)
  providers/       open_meteo, lrclib, tmdb (TVmaze + TMDB + iTunes)
  bridge.rs        token-gated LAN HTTP (tiny_http), per-IP rate limiting
tools/             icon generator, SMTC probe, capture/UIA diagnostics
```

Windows-rs COM interfaces are not `Send`, so every WinRT interaction runs on
blocking worker threads (`tauri::async_runtime::spawn_blocking`) with MTA COM
initialized; the SMTC manager is re-acquired per work item (it is a
per-process singleton).

## Tests

```powershell
cargo test --manifest-path src-tauri/Cargo.toml   # 29 unit tests
npm run test:fe                                   # vitest: spring engine + island render
```

## License

MIT
