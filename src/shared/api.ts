import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Anchor,
  BtDevice,
  ClipEntry,
  ExpandMode,
  GeoResult,
  LyricsPayload,
  MediaInfo,
  MediaState,
  MixerApp,
  NotifyItem,
  Reminder,
  SearchHit,
  Settings,
  Theme,
  Timer,
  WeatherData,
} from "./types";

// Event channel names — keep in sync with src-tauri/src/events.rs.
export const ev = {
  MEDIA: "media:update",
  MEDIA_INFO: "media:info",
  SPECTRUM: "audio:spectrum",
  KEEPAWAKE: "keepawake:state",
  SETTINGS: "settings:changed",
  REMINDERS_SYNC: "reminders:sync",
  REMINDER_FIRED: "reminder:fired",
  VOLUME_OSD: "volume:osd",
  OSD_KEY: "osd:key",
  BT_BATTERY: "bt:battery",
  METRICS: "metrics:sample",
  TIMER_TICK: "timer:tick",
  TIMER_DONE: "timer:done",
  CLIPBOARD_SYNC: "clipboard:sync",
  NOTIFY_NEW: "notify:new",
  BT_DEVICE: "bt:device",
  WEATHER: "weather:update",
  FULLSCREEN: "fullscreen:changed",
  DISPLAY: "display:changed",
  ATTENTION: "island:attention",
} as const;

// ---- island window ----
export const islandSetFrame = (x: number, y: number, w: number, h: number, radius: number) =>
  invoke("island_set_frame", { x, y, w, h, radius });
export const islandExpand = () => invoke("island_expand");
export const islandCollapse = () => invoke("island_collapse");
export const openSettings = () => invoke("open_settings");
export const quitApp = () => invoke("quit_app");
export const islandAnchor = () => invoke<Anchor>("island_anchor");
export interface AwakeState {
  active: boolean;
  untilMs: number;
  remainingMs: number;
}
export const keepAwakeSet = (minutes: number) =>
  invoke<number>("keep_awake_set", { minutes });
export const keepAwakeOff = () => invoke("keep_awake_off");
export const micState = () => invoke<boolean>("mic_state");
export const micSetMute = (muted: boolean) =>
  invoke<boolean>("mic_set_mute_state", { muted });
export const noteGet = () => invoke<string>("note_get");
export const noteSave = (text: string) => invoke("note_save", { text });
export const focusStats = () =>
  invoke<[string, number][]>("focus_stats");
export const remindersList = () =>
  invoke<Reminder[]>("reminders_list");
export const reminderAdd = (text: string, intervalMin: number) =>
  invoke<number>("reminder_add", { text, intervalMin });
export const reminderDelete = (id: number) =>
  invoke("reminder_delete", { id });
export const mixerList = () =>
  invoke<MixerApp[]>("mixer_list");
export const mixerSet = (pid: number, volume: number) =>
  invoke("mixer_set", { pid, volume });
export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
}
export const audioDevices = () =>
  invoke<AudioDevice[]>("audio_devices");
export const deviceSetDefault = (id: string) =>
  invoke("device_set_default", { id });
export const frontendLog = (message: string) =>
  invoke("frontend_log", { message }).catch(() => {});
export const updateCheck = () =>
  invoke<{ available: boolean; version?: string; notes?: string }>(
    "update_check",
  );
export const updateInstall = () => invoke<void>("update_install");
/** Fetch a remote image through Rust as a data URL (CORS-safe canvas sampling). */
export const fetchDataUrl = (url: string) =>
  invoke<string | null>("fetch_data_url", { url }).catch(() => null);

// ---- settings ----
export const settingsGet = () => invoke<Settings>("settings_get");
export const settingsUpdate = (patch: Partial<Settings>) =>
  invoke<Settings>("settings_update", { patch });
export const themeSetting = () => invoke<Theme>("theme_setting");
export const expandModeSetting = () => invoke<ExpandMode>("expand_mode_setting");

// ---- media ----
export const mediaCurrent = () => invoke<MediaState | null>("media_current");
export const copyNowPlaying = () =>
  invoke<boolean>("copy_now_playing");
export const mediaPlayPause = () => invoke("media_play_pause");
export const mediaNext = () => invoke("media_next");
export const mediaPrev = () => invoke("media_prev");
// SMTC (windows-rs 0.62) exposes no timeline-change API → no seek.

// ---- timers ----
export const timerAdd = (label: string, seconds: number) =>
  invoke<number>("timer_add", { label, seconds });
export const timerCancel = (id: number) => invoke<boolean>("timer_cancel", { id });
export const timerPause = (id: number) => invoke<boolean>("timer_pause", { id });
export const timerResume = (id: number) => invoke<boolean>("timer_resume", { id });
export const timersList = () => invoke<Timer[]>("timers_list");

// ---- metrics ----
export const metricsSubscribe = (enable: boolean) =>
  invoke("metrics_subscribe", { enable });

// ---- weather ----
export const weatherGeocode = (query: string) =>
  invoke<GeoResult[]>("weather_geocode", { query });
export const weatherRefresh = () =>
  invoke<WeatherData | { error: string }>("weather_refresh");
export const weatherSetCity = (geo: GeoResult) =>
  invoke<Settings>("weather_set_city", { city: geo });

// ---- lyrics ----
export const lyricsFetch = (
  artist: string,
  title: string,
  durationS: number,
  album: string,
) =>
  invoke<LyricsPayload | null>("lyrics_fetch", {
    artist,
    title,
    durationS,
    album,
  });

// ---- media info ----
export const mediaLookup = (kind: string, query?: string) =>
  invoke<MediaInfo | null>("media_lookup", { kind, query: query ?? null });
export const mediaSearch = (kind: string, query: string) =>
  invoke<SearchHit[]>("media_search", { kind, query });
export const mediaInfoCached = () =>
  invoke<MediaInfo | null>("media_info_cached");

// ---- clipboard ----
export const clipboardList = () => invoke<ClipEntry[]>("clipboard_list");
export const clipboardReveal = (id: number) =>
  invoke<string | null>("clipboard_reveal", { id });
export const clipboardCopy = (id: number) =>
  invoke<boolean>("clipboard_copy", { id });
export const clipboardDelete = (id: number) =>
  invoke<boolean>("clipboard_delete", { id });
export const clipboardClear = () => invoke("clipboard_clear");

// ---- notifications ----
export const notificationsList = () => invoke<NotifyItem[]>("notifications_list");
export const notificationsClear = () => invoke("notifications_clear");

// ---- bluetooth ----
export const btDevices = () => invoke<BtDevice[]>("bt_devices");

// ---- bridge ----
export interface BridgeStatus {
  enabled: boolean;
  token: string;
  port: number;
}
export const bridgeStatus = () => invoke<BridgeStatus>("bridge_status");
export const bridgeSetEnabled = (enable: boolean) =>
  invoke<BridgeStatus>("bridge_set_enabled", { enable });
export const bridgeRegenerateToken = () =>
  invoke<BridgeStatus>("bridge_regenerate_token");

// ---- providers ----
export const providersSetTmdb = (key: string) =>
  invoke("providers_set_tmdb", { key });
export const providersClearTmdb = () => invoke("providers_clear_tmdb");
export const providersHasTmdb = () => invoke<boolean>("providers_has_tmdb");

export { listen };
