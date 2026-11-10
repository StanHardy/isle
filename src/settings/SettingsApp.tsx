import { useEffect, useState } from "react";
import * as api from "../shared/api";
import type { GeoResult, Settings as SettingsType } from "../shared/types";

type Page = "island" | "modules" | "media" | "privacy" | "phone" | "about";

const PAGES: { id: Page; label: string }[] = [
  { id: "island", label: "Island" },
  { id: "modules", label: "Modules" },
  { id: "media", label: "Media & weather" },
  { id: "privacy", label: "Privacy" },
  { id: "phone", label: "Phone companion" },
  { id: "about", label: "About" },
];

function Toggle({
  checked,
  onChange,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  hint?: string;
}) {
  return (
    <label className="setting-row">
      <span className="setting-text">
        <span className="setting-label">{label}</span>
        {hint && <span className="setting-hint">{hint}</span>}
      </span>
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
    </label>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="settings-section">
      <h2>{title}</h2>
      <div className="settings-group">{children}</div>
    </section>
  );
}

export default function SettingsApp() {
  const [settings, setSettings] = useState<SettingsType | null>(null);
  const [page, setPage] = useState<Page>("island");
  const [tmdbKey, setTmdbKey] = useState("");
  const [tmdbMsg, setTmdbMsg] = useState<string | null>(null);
  const [bridge, setBridge] = useState<api.BridgeStatus | null>(null);
  const [cityQuery, setCityQuery] = useState("");
  const [cityResults, setCityResults] = useState<GeoResult[] | null>(null);

  useEffect(() => {
    api.settingsGet().then(setSettings).catch(console.error);
    api.bridgeStatus().then(setBridge).catch(() => {});
  }, []);

  if (!settings) return <div className="settings-loading">Loading…</div>;

  const update = (patch: Partial<SettingsType>) => {
    api.settingsUpdate(patch).then(setSettings).catch(console.error);
  };

  return (
    <div className="settings-app">
      <aside className="settings-nav">
        <div className="settings-brand">Isle</div>
        <nav>
          {PAGES.map((p) => (
            <button
              key={p.id}
              type="button"
              className={page === p.id ? "nav-item active" : "nav-item"}
              onClick={() => setPage(p.id)}
            >
              {p.label}
            </button>
          ))}
        </nav>
      </aside>

      <main className="settings-main">
        {page === "island" && (
          <>
            <Section title="Behavior">
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Island design</span>
                  <span className="setting-hint">
                    Pill: floating glass capsule · Notch: opaque, anchored to the screen edge ·
                    Black / Clear: Droppy-style material variants
                  </span>
                </span>
                <select
                  value={settings.islandDesign}
                  onChange={(e) =>
                    update({ islandDesign: e.target.value as SettingsType["islandDesign"] })
                  }
                >
                  <option value="pill">Pill</option>
                  <option value="notch">Notch</option>
                  <option value="black">Black</option>
                  <option value="clear">Clear</option>
                </select>
              </label>
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Expand on</span>
                  <span className="setting-hint">Hover dwell or explicit click</span>
                </span>
                <select
                  value={settings.expandMode}
                  onChange={(e) => update({ expandMode: e.target.value as SettingsType["expandMode"] })}
                >
                  <option value="hover">Hover</option>
                  <option value="click">Click</option>
                </select>
              </label>
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Scale</span>
                  <span className="setting-hint">Island size multiplier</span>
                </span>
                <input
                  type="range"
                  min="0.6"
                  max="1.5"
                  step="0.05"
                  value={settings.islandScale}
                  onChange={(e) => update({ islandScale: Number(e.target.value) })}
                />
              </label>
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Theme</span>
                </span>
                <select
                  value={settings.theme}
                  onChange={(e) => update({ theme: e.target.value as SettingsType["theme"] })}
                >
                  <option value="system">System</option>
                  <option value="dark">Dark</option>
                  <option value="light">Light</option>
                </select>
              </label>
              <Toggle
                label="Hide in fullscreen apps"
                hint="The island steps aside during games and fullscreen video"
                checked={settings.hideFullscreen}
                onChange={(v) => update({ hideFullscreen: v })}
              />
            </Section>
          </>
        )}

        {page === "modules" && (
          <Section title="Modules">
            <p className="setting-hint">
              Choose which cards appear in the expanded panel and their order.
            </p>
            {settings.moduleOrder.map((id, idx) => (
              <div key={id} className="setting-row module-row">
                <span className="setting-label">{id}</span>
                <span className="module-controls">
                  <button
                    type="button"
                    className="mini-btn"
                    aria-label={`Move ${id} up`}
                    disabled={idx === 0}
                    onClick={() => {
                      const order = [...settings.moduleOrder];
                      [order[idx - 1], order[idx]] = [order[idx], order[idx - 1]];
                      update({ moduleOrder: order });
                    }}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="mini-btn"
                    aria-label={`Move ${id} down`}
                    disabled={idx === settings.moduleOrder.length - 1}
                    onClick={() => {
                      const order = [...settings.moduleOrder];
                      [order[idx + 1], order[idx]] = [order[idx], order[idx + 1]];
                      update({ moduleOrder: order });
                    }}
                  >
                    ↓
                  </button>
                  <input
                    type="checkbox"
                    role="switch"
                    aria-label={`Toggle ${id}`}
                    checked={settings.modulesEnabled[id] ?? false}
                    onChange={(e) =>
                      update({
                        modulesEnabled: {
                          ...settings.modulesEnabled,
                          [id]: e.target.checked,
                        },
                      })
                    }
                  />
                </span>
              </div>
            ))}
          </Section>
        )}

        {page === "media" && (
          <>
            <Section title="Playback">
              <Toggle
                label="Media (SMTC)"
                hint="Album art, transport controls, now playing"
                checked={settings.mediaEnabled}
                onChange={(v) => update({ mediaEnabled: v })}
              />
              <Toggle
                label="Lyrics"
                hint="Line-synced lyrics via LRCLIB (keyless)"
                checked={settings.lyricsEnabled}
                onChange={(v) => update({ lyricsEnabled: v })}
              />
              <Toggle
                label="Auto-lookup TV/movie info"
                hint="Fetch art + summaries in the background for video-looking titles (episodes, year-tagged movies)"
                checked={settings.mediaInfoAuto}
                onChange={(v) => update({ mediaInfoAuto: v })}
              />
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Spectrum style</span>
                  <span className="setting-hint">
                    Bar colors: app accent, white, rainbow gradient, album tint, or muted mono
                  </span>
                </span>
                <select
                  value={settings.spectrumStyle}
                  onChange={(e) =>
                    update({ spectrumStyle: e.target.value as SettingsType["spectrumStyle"] })
                  }
                >
                  <option value="accent">Accent</option>
                  <option value="white">White</option>
                  <option value="gradient">Gradient</option>
                  <option value="tint">Album tint</option>
                  <option value="mono">Mono</option>
                </select>
              </label>
              <Toggle
                label="System metrics"
                hint="CPU, memory and network sampling while the panel is open"
                checked={settings.metricsEnabled}
                onChange={(v) => update({ metricsEnabled: v })}
              />
              <Toggle
                label="Bluetooth audio events"
                hint="Connection animations and device list"
                checked={settings.bluetoothEnabled}
                onChange={(v) => update({ bluetoothEnabled: v })}
              />
            </Section>
            <Section title="Weather">
              <Toggle
                label="Weather"
                hint="Open-Meteo current conditions (keyless)"
                checked={settings.weatherEnabled}
                onChange={(v) => update({ weatherEnabled: v })}
              />
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Temperature</span>
                </span>
                <select
                  value={settings.tempUnit}
                  onChange={(e) => update({ tempUnit: e.target.value as SettingsType["tempUnit"] })}
                >
                  <option value="metric">Celsius</option>
                  <option value="imperial">Fahrenheit</option>
                </select>
              </label>
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Wind speed</span>
                </span>
                <select
                  value={settings.windUnit}
                  onChange={(e) => update({ windUnit: e.target.value as SettingsType["windUnit"] })}
                >
                  <option value="kmh">km/h</option>
                  <option value="mph">mph</option>
                </select>
              </label>
              <div className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">City</span>
                  <span className="setting-hint">
                    {settings.weatherCity
                      ? `${settings.weatherCity.name} (${settings.weatherCity.lat.toFixed(2)}, ${settings.weatherCity.lon.toFixed(2)})`
                      : "None selected"}
                  </span>
                </span>
              </div>
              <form
                className="inline-form"
                onSubmit={async (e) => {
                  e.preventDefault();
                  setCityResults(await api.weatherGeocode(cityQuery));
                }}
              >
                <input
                  type="text"
                  placeholder="Search city"
                  aria-label="Search city"
                  value={cityQuery}
                  onChange={(e) => setCityQuery(e.target.value)}
                />
                <button type="submit" className="chip">Search</button>
              </form>
              {cityResults && (
                <ul className="city-results">
                  {cityResults.map((r) => (
                    <li key={`${r.lat},${r.lon}`}>
                      <button
                        type="button"
                        className="mini-btn wide"
                        onClick={async () => {
                          const next = await api.weatherSetCity(r);
                          setSettings(next);
                          setCityResults(null);
                          setCityQuery("");
                        }}
                      >
                        {r.name}
                        {r.admin1 ? `, ${r.admin1}` : ""}
                        {r.country ? ` (${r.country})` : ""}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
            <Section title="TV & movie info">
              <p className="setting-hint">
                TV art and episode summaries come from TVmaze (keyless). Movies use TMDB when you
                provide a free API key (stored in Windows Credential Manager), otherwise a keyless
                iTunes fallback.
              </p>
              <div className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">TMDB key</span>
                  <span className="setting-hint">
                    {settings.tmdbConfigured ? "Configured ✓" : "Not configured"}
                  </span>
                </span>
              </div>
              <form
                className="inline-form"
                onSubmit={async (e) => {
                  e.preventDefault();
                  if (!tmdbKey.trim()) return;
                  await api.providersSetTmdb(tmdbKey);
                  const next = await api.settingsGet();
                  setSettings(next);
                  setTmdbKey("");
                  setTmdbMsg("Saved to Windows Credential Manager.");
                }}
              >
                <input
                  type="password"
                  placeholder="TMDB API key (v3)"
                  aria-label="TMDB API key"
                  value={tmdbKey}
                  onChange={(e) => setTmdbKey(e.target.value)}
                />
                <button type="submit" className="chip">Save</button>
                {settings.tmdbConfigured && (
                  <button
                    type="button"
                    className="chip"
                    onClick={async () => {
                      await api.providersClearTmdb();
                      const next = await api.settingsGet();
                      setSettings(next);
                      setTmdbMsg("Key removed.");
                    }}
                  >
                    Remove
                  </button>
                )}
              </form>
              {tmdbMsg && <p className="setting-hint">{tmdbMsg}</p>}
            </Section>
          </>
        )}

        {page === "privacy" && (
          <>
            <Section title="Local data">
              <Toggle
                label="Clipboard history"
                hint="In-memory only, bounded list, sensitive entries masked. Never leaves this machine."
                checked={settings.clipboardEnabled}
                onChange={(v) => update({ clipboardEnabled: v })}
              />
              <label className="setting-row">
                <span className="setting-text">
                  <span className="setting-label">Clipboard retention</span>
                  <span className="setting-hint">Maximum stored entries (10–500)</span>
                </span>
                <input
                  type="number"
                  min="10"
                  max="500"
                  value={settings.clipboardKeep}
                  onChange={(e) => update({ clipboardKeep: Number(e.target.value) })}
                />
              </label>
              <Toggle
                label="Notification mirroring"
                hint="Reads toast banner text via UI Automation while banners show. Opt-in."
                checked={settings.notificationsEnabled}
                onChange={(v) => update({ notificationsEnabled: v })}
              />
              <div className="setting-row">
                <button
                  type="button"
                  className="chip"
                  onClick={async () => {
                    await api.clipboardClear();
                    await api.notificationsClear();
                  }}
                >
                  Clear clipboard + notification data now
                </button>
              </div>
            </Section>
            <Section title="Network">
              <p className="setting-hint">
                Isle contacts Open-Meteo (weather), LRCLIB (lyrics), TVmaze / TMDB / iTunes (media
                info) only for the features you enable. No telemetry, no analytics, no accounts.
              </p>
            </Section>
          </>
        )}

        {page === "phone" && (
          <>
            <Section title="Phone companion bridge">
              <Toggle
                label="Enable bridge"
                hint="LAN HTTP endpoint (0.0.0.0:7350) — Windows Firewall prompts on first enable"
                checked={settings.bridgeEnabled}
                onChange={(v) => {
                  api.bridgeSetEnabled(v).then(setBridge).catch(console.error);
                }}
              />
              <p className="setting-hint">
                Pair a phone by posting JSON to <code>http://this-pc:7350/notify</code> with your
                token (works with Tasker, HTTP Shortcuts, MacroDroid, or any automation app that
                can POST). Payload:
                <code>{`{"token":"…","app":"WhatsApp","title":"…","body":"…"}`}</code>
              </p>
              {bridge && (
                <div className="setting-row token-row">
                  <code className="bridge-token">{bridge.token}</code>
                  <button
                    type="button"
                    className="chip"
                    onClick={async () => {
                      const next = await api.bridgeRegenerateToken();
                      setBridge(next);
                    }}
                  >
                    Regenerate
                  </button>
                </div>
              )}
              <p className="setting-hint">
                Supported: notification relay to the island, dismiss from the feed. Not supported:
                replying from the PC — that requires a device-side agent and is deliberately not
                promised by this bridge.
              </p>
            </Section>
          </>
        )}

        {page === "about" && (
          <>
            <Section title="About Isle">
              <p className="setting-hint">
                Isle 1.0.0 — a local-first Dynamic Island for Windows 11. Built with Tauri, Rust
                and React. Media via GlobalSystemMediaTransportControls; device events via
                Windows.Devices.Enumeration; secrets in Windows Credential Manager.
              </p>
              <div className="setting-row">
                <button type="button" className="chip" onClick={() => api.quitApp()}>
                  Quit Isle
                </button>
              </div>
            </Section>
          </>
        )}
      </main>
    </div>
  );
}
