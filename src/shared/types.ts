// Mirrors the serde payloads in src-tauri (camelCase on both sides).

export interface MediaState {
  id: string;
  app: string;
  title: string;
  /** Untouched SMTC title (filenames for local players) — not for display. */
  rawTitle: string;
  artist: string;
  album: string;
  status: string;
  positionMs: number;
  durationMs: number;
  artKey: string;
  artB64: string | null;
  canPlay: boolean;
  canPause: boolean;
  canNext: boolean;
  canPrev: boolean;
  shuffle: boolean | null;
  repeat: string | null;
}

export interface Timer {
  id: number;
  label: string;
  totalMs: number;
  remainingMs: number;
  paused: boolean;
  endsAtMs: number;
}

export interface MetricSample {
  cpu: number;
  memUsed: number;
  memTotal: number;
  netUpBps: number;
  netDownBps: number;
}

export interface ClipEntry {
  id: number;
  text: string;
  tsMs: number;
  sensitive: boolean;
}

export interface NotifyItem {
  id: number;
  source: string;
  app: string;
  title: string;
  body: string;
  tsMs: number;
}

export interface BtDevice {
  id: string;
  name: string;
  connected: boolean;
}

export interface GeoResult {
  name: string;
  admin1: string | null;
  country: string | null;
  lat: number;
  lon: number;
}

export interface WeatherData {
  city: string;
  temp: number;
  code: number;
  wind: number;
  humidity: number;
  hi: number;
  lo: number;
  precipProb: number;
  isDay: boolean;
  fetchedMs: number;
}

export interface LyricLine {
  tMs: number;
  text: string;
}

export interface LyricsPayload {
  synced: LyricLine[];
  plain: string | null;
  provider: string;
}

export interface MediaInfo {
  title: string;
  overview: string;
  poster: string;
  provider: string;
}

export interface SearchHit {
  title: string;
  year: string;
  overview: string;
  poster: string;
  provider: string;
}

export interface CityRef {
  name: string;
  lat: number;
  lon: number;
}

export type TempUnit = "metric" | "imperial";
export type WindUnit = "kmh" | "mph";
export type ExpandMode = "hover" | "click";
export type Theme = "system" | "dark" | "light";

export interface Settings {
  version: number;
  islandScale: number;
  expandMode: ExpandMode;
  theme: Theme;
  hideFullscreen: boolean;
  moduleOrder: string[];
  modulesEnabled: Record<string, boolean>;
  mediaEnabled: boolean;
  metricsEnabled: boolean;
  weatherEnabled: boolean;
  weatherCity: CityRef | null;
  tempUnit: TempUnit;
  windUnit: WindUnit;
  lyricsEnabled: boolean;
  /** Auto-lookup TV/movie info for video-looking media. */
  mediaInfoAuto: boolean;
  clipboardEnabled: boolean;
  clipboardKeep: number;
  notificationsEnabled: boolean;
  bluetoothEnabled: boolean;
  bridgeEnabled: boolean;
  bridgeToken: string;
  tmdbConfigured: boolean;
}

export interface Anchor {
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
}

export interface Attention {
  kind: string;
  name?: string;
  label?: string;
  title?: string;
  text?: string;
}
