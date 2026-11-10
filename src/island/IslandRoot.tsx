import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import * as api from "../shared/api";
import type {
  Anchor,
  Attention,
  MediaInfo,
  MediaState,
  Settings,
  Timer,
} from "../shared/types";
import { Spring, prefersReducedMotion } from "../shared/spring";
import SpectrumBars from "../shared/SpectrumBars";
import MediaModule from "./modules/MediaModule";
import TimersModule from "./modules/TimersModule";
import WeatherModule from "./modules/WeatherModule";
import MetricsModule from "./modules/MetricsModule";
import { BluetoothModule, ClipboardModule, NotificationsModule } from "./modules/PrivacyModules";
import {
  FocusModule,
  InfoModule,
  LyricsModule,
  MicModule,
  NoteModule,
  RemindersModule,
} from "./modules/ExtrasModules";

const COMPACT = { w: 348, h: 56 };
const EXPANDED = { w: 424, h: 580 };
// Flush against the top edge — any gap lets maximized apps' title bars
// peek through above the island.
const TOP_OFFSET = 0;
const RADIUS = { compact: 28, expanded: 24 };

/**
 * Compact geometry per island design. The notch hangs from the screen edge:
 * its window top is pushed above the screen by the corner radius, so the
 * uniform rounded region shows only the bottom corners.
 */
const DESIGN_METRICS: Record<
  string,
  { w: number; h: number; yOff: number; radius: number }
> = {
  pill: { w: 348, h: 56, yOff: TOP_OFFSET, radius: 28 },
  notch: { w: 360, h: 80, yOff: -24, radius: 24 },
  black: { w: 348, h: 56, yOff: TOP_OFFSET, radius: 28 },
  clear: { w: 348, h: 56, yOff: TOP_OFFSET, radius: 28 },
  // Full-width strip: w comes from the work area; radius 0 = rectangular.
  strip: { w: 0, h: 48, yOff: 0, radius: 0 },
};

type Mode = "compact" | "expanded";

/** Bento layout: wide cards span both columns, narrow cards pair up. */
const WIDE_MODULES = new Set([
  "media",
  "info",
  "metrics",
  "lyrics",
  "notifications",
  "clipboard",
]);

interface IslandCtx {
  settings: Settings | null;
  update: (patch: Partial<Settings>) => void;
  media: MediaState | null;
  timers: Timer[];
}

export const IslandContext = createContext<IslandCtx>({
  settings: null,
  update: () => {},
  media: null,
  timers: [],
});

export const useIsland = () => useContext(IslandContext);

/**
 * Sample a data-URL image for its dominant VIVID color: 16×16 downscale,
 * average only the saturated-enough pixels. Covers that are mostly black
 * with colored text (TV posters) still yield their accent color, while a
 * global 1×1 average would read as desaturated grey. Grey art yields null
 * (bars fall back to the accent color).
 */
function sampleTint(dataUrl: string): Promise<string | null> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      try {
        const N = 16;
        const c = document.createElement("canvas");
        c.width = N;
        c.height = N;
        const ctx = c.getContext("2d");
        if (!ctx) return resolve(null);
        ctx.drawImage(img, 0, 0, N, N);
        const d = ctx.getImageData(0, 0, N, N).data;
        let r = 0;
        let g = 0;
        let b = 0;
        let n = 0;
        for (let i = 0; i < N * N; i++) {
          const R = d[i * 4];
          const G = d[i * 4 + 1];
          const B = d[i * 4 + 2];
          const mx = Math.max(R, G, B);
          const mn = Math.min(R, G, B);
          const s = mx === 0 ? 0 : (mx - mn) / mx;
          if (s >= 0.25 && mx > 40) {
            r += R;
            g += G;
            b += B;
            n++;
          }
        }
        if (n < 4) return resolve(null);
        const ar = r / n;
        const ag = g / n;
        const ab = b / n;
        const mx = Math.max(ar, ag, ab);
        const mn = Math.min(ar, ag, ab);
        const sat = mx === 0 ? 0 : (mx - mn) / mx;
        if (sat < 0.18) return resolve(null);
        const mix = (v: number) => Math.round(v + (255 - v) * 0.35);
        resolve(`rgb(${mix(ar)}, ${mix(ag)}, ${mix(ab)})`);
      } catch {
        resolve(null);
      }
    };
    img.onerror = () => resolve(null);
    img.src = dataUrl;
  });
}

/** SMTC artwork cache keyed by media key; tint sampled from the art. */
function useArtCache(media: MediaState | null) {
  const cache = useRef(new Map<string, string>());
  const [url, setUrl] = useState<string | null>(null);
  const [tint, setTint] = useState<string | null>(null);

  useEffect(() => {
    if (!media) {
      setUrl(null);
      setTint(null);
      return;
    }
    if (media.artB64) cache.current.set(media.artKey, media.artB64);
    const u = cache.current.get(media.artKey) ?? null;
    setUrl(u);
    if (!u) {
      setTint(null);
      return;
    }
    let cancelled = false;
    sampleTint(u).then((t) => {
      if (!cancelled) setTint(t);
    });
    return () => {
      cancelled = true;
    };
  }, [media?.artKey, media?.artB64]);

  return { url, tint };
}

function Clock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(t);
  }, []);
  const time = now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const date = now.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
  return (
    <div className="clock">
      <span className="clock-time">{time}</span>
      <span className="clock-date">{date}</span>
    </div>
  );
}

function fmtCountdown(ms: number) {
  const s = Math.max(0, Math.ceil(ms / 1000));
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${String(r).padStart(2, "0")}`;
}

/** Ticking clock text — inline new Date() only refreshes on re-renders. */
function MiniTime() {
  const [t, setT] = useState(() => new Date());
  useEffect(() => {
    const iv = setInterval(() => setT(new Date()), 10_000);
    return () => clearInterval(iv);
  }, []);
  return <>{t.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</>;
}

export default function IslandRoot() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [mode, setMode] = useState<Mode>("compact");
  const [media, setMedia] = useState<MediaState | null>(null);
  const [timers, setTimers] = useState<Timer[]>([]);
  const [attention, setAttention] = useState<Attention | null>(null);
  const [notifBadge, setNotifBadge] = useState(0);
  const [btFlash, setBtFlash] = useState<{ name: string; connected: boolean } | null>(null);
  const [fsHidden, setFsHidden] = useState(false);
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  const [info, setInfo] = useState<{ key: string; poster: string } | null>(null);
  const [micMuted, setMicMuted] = useState(false);

  useEffect(() => {
    const poll = () =>
      api
        .micState()
        .then((m) => setMicMuted(m))
        .catch(() => {});
    poll();
    const iv = setInterval(poll, 3000);
    return () => clearInterval(iv);
  }, []);

  const wSpring = useRef(new Spring(COMPACT.w, { stiffness: 210, damping: 24 }));
  const hSpring = useRef(new Spring(COMPACT.h, { stiffness: 210, damping: 24 }));
  const rafRef = useRef<number | null>(null);
  const hoverTimer = useRef<number | null>(null);
  const leaveTimer = useRef<number | null>(null);

  const update = useCallback((patch: Partial<Settings>) => {
    api.settingsUpdate(patch).then(setSettings).catch(console.error);
  }, []);

  // Initial state + subscriptions.
  useEffect(() => {
    api.settingsGet().then((s) => setSettings(s ?? null)).catch(console.error);
    api.mediaCurrent().then((m) => setMedia(m ?? null)).catch(() => {});
    api.timersList().then((t) => setTimers(t ?? [])).catch(() => {});
    api.islandAnchor().then((a) => setAnchor(a ?? null)).catch(() => {});

    const unsubs = [
      api.listen<MediaState | null>(api.ev.MEDIA, (e) => {
        setMedia(e.payload);
      }),
      api.listen<{ key: string; info: MediaInfo }>(api.ev.MEDIA_INFO, (e) => {
        if (e.payload.info.poster) {
          setInfo({ key: e.payload.key, poster: e.payload.info.poster });
        }
      }),
      api.listen<Timer[]>(api.ev.TIMER_TICK, (e) => setTimers(e.payload ?? [])),
      api.listen<{ id: number; label: string }>(api.ev.TIMER_DONE, (e) => {
        setAttention({ kind: "timer-done", label: e.payload.label });
        setTimers((t) => (t ?? []).filter((x) => x.id !== e.payload.id));
      }),
      api.listen<{ name: string; connected: boolean }>(api.ev.BT_DEVICE, (e) => {
        setBtFlash(e.payload);
      }),
      api.listen<unknown>(api.ev.NOTIFY_NEW, () => setNotifBadge((n) => n + 1)),
      api.listen<{ active: boolean }>(api.ev.FULLSCREEN, (e) => {
        setFsHidden(e.payload.active);
        if (e.payload.active) setMode("compact");
      }),
      api.listen<Anchor>(api.ev.DISPLAY, (e) => setAnchor(e.payload)),
      api.listen<Settings>(api.ev.SETTINGS, (e) => setSettings(e.payload)),
      api.listen<Attention>(api.ev.ATTENTION, (e) => setAttention(e.payload)),
    ];
    return () => {
      unsubs.forEach((u) => {
        u.then((f) => f()).catch(() => {});
      });
    };
  }, []);

  // Poster lookup cache for the current track (used as artwork fallback).
  useEffect(() => {
    if (!media) {
      setInfo(null);
      return;
    }
    api
      .mediaInfoCached()
      .then((cached) => {
        if (cached?.poster) {
          setInfo({ key: media.artKey, poster: cached.poster });
        }
      })
      .catch(() => {});
  }, [media?.artKey]);

  // Transient attention clears itself.
  useEffect(() => {
    if (!attention) return;
    const t = setTimeout(() => setAttention(null), 2800);
    return () => clearTimeout(t);
  }, [attention]);

  useEffect(() => {
    if (!btFlash) return;
    const t = setTimeout(() => setBtFlash(null), 4000);
    return () => clearTimeout(t);
  }, [btFlash]);

  const expand = useCallback(() => {
    if (leaveTimer.current) window.clearTimeout(leaveTimer.current);
    if (hoverTimer.current) window.clearTimeout(hoverTimer.current);
    setMode("expanded");
    setNotifBadge(0);
    api.islandExpand().catch(() => {});
  }, []);

  const collapse = useCallback(() => {
    setMode("compact");
    api.islandCollapse().catch(() => {});
  }, []);

  // Window frame follows the springs. Strip design: full work-area width,
  // flush at the top; expanded adds the panel below the strip.
  useEffect(() => {
    const designId = settings?.islandDesign ?? "pill";
    const design = DESIGN_METRICS[designId] ?? DESIGN_METRICS.pill;
    const compact = mode === "compact";
    const isStrip = designId === "strip";
    const scaleF = settings?.islandScale ?? 1;
    const workW = anchor ? anchor.width / (anchor.scale || 1) : EXPANDED.w;

    let targetW: number;
    let targetH: number;
    if (isStrip) {
      targetW = workW;
      targetH = compact ? design.h : design.h + EXPANDED.h;
    } else {
      targetW = (compact ? design.w : EXPANDED.w) * scaleF;
      targetH = (compact ? design.h : EXPANDED.h) * scaleF;
    }
    wSpring.current.set(targetW);
    hSpring.current.set(targetH);
    const radius = isStrip ? 0 : compact ? design.radius : RADIUS.expanded;
    const yOff = compact ? design.yOff : 0;

    const writeFrame = () => {
      const a = anchor;
      if (!a) return;
      const scale = a.scale || 1;
      const w = wSpring.current.value;
      const h = hSpring.current.value;
      const x = isStrip
        ? a.x / scale
        : a.x / scale + (a.width / scale - w) / 2;
      const y = a.y / scale + yOff;
      api.islandSetFrame(x, y, w, h, radius).catch(() => {});
    };

    if (prefersReducedMotion()) {
      wSpring.current.snap(targetW);
      hSpring.current.snap(targetH);
      writeFrame();
      return;
    }
    const tick = () => {
      const settledW = wSpring.current.step(1 / 60);
      const settledH = hSpring.current.step(1 / 60);
      writeFrame();
      if (settledW && settledH) {
        rafRef.current = null;
        return;
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    };
  }, [mode, settings?.islandScale, settings?.islandDesign, anchor]);

  const activeTimer = timers.find((t) => !t.paused);
  const pillKind = useMemo(() => {
    if (attention) return "attention";
    if (activeTimer) return "timer";
    if (btFlash) return "bt";
    if (media?.status === "playing") return "media";
    if (media && media.title) return "media-dim";
    if (notifBadge > 0) return "notify";
    return "clock";
  }, [attention, activeTimer, btFlash, media, notifBadge]);

  const onPillEnter = () => {
    // A full-width strip is far too big a hover target for dwell-expand.
    if (settings?.islandDesign === "strip") return;
    if (mode !== "compact" || settings?.expandMode !== "hover") return;
    hoverTimer.current = window.setTimeout(expand, 160);
  };
  const onPillLeave = () => {
    if (hoverTimer.current) window.clearTimeout(hoverTimer.current);
    if (mode !== "expanded") return;
    leaveTimer.current = window.setTimeout(collapse, 350);
  };
  const onPanelEnter = () => {
    if (leaveTimer.current) window.clearTimeout(leaveTimer.current);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && mode === "expanded") collapse();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode, collapse]);

  const ctx: IslandCtx = { settings, update, media, timers };
  const { url: smtcArt, tint: smtcTint } = useArtCache(media);

  // Artwork: SMTC art first; the looked-up TV/movie poster is a graceful
  // fallback for local players that expose no thumbnail.
  const posterForTrack =
    media && info && info.key === media.artKey ? info.poster : null;
  const art = smtcArt ?? posterForTrack;

  // Tint follows whatever artwork is DISPLAYED: SMTC art is sampled in the
  // hook; the poster is fetched through Rust as a data URL so the canvas
  // sample isn't CORS-tainted on remote images.
  const [posterTint, setPosterTint] = useState<string | null>(null);
  const posterTintCache = useRef(new Map<string, string>());
  useEffect(() => {
    if (!posterForTrack || smtcArt) {
      setPosterTint(null);
      return;
    }
    const cached = posterTintCache.current.get(posterForTrack);
    if (cached !== undefined) {
      setPosterTint(cached || null);
      return;
    }
    let cancelled = false;
    api
      .fetchDataUrl(posterForTrack)
      .then((dataUrl) => {
        if (cancelled || !dataUrl) return;
        posterTintCache.current.set(posterForTrack, ""); // dedupe in-flight
        return sampleTint(dataUrl).then((t) => {
          if (cancelled) return;
          if (t) posterTintCache.current.set(posterForTrack, t);
          setPosterTint(t);
        });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [posterForTrack, smtcArt]);
  const tint = smtcArt ? smtcTint : posterTint;
  const tintLive = media?.status === "playing" && tint ? tint : null;

  if (fsHidden) {
    return (
      <IslandContext.Provider value={ctx}>
        <div className="island-root hidden-host" />
      </IslandContext.Provider>
    );
  }

  const renderModule = (id: string) => {
    switch (id) {
      case "media":
        return settings?.mediaEnabled ? <MediaModule key={id} art={art} tint={tint} /> : null;
      case "timer":
        return <TimersModule key={id} />;
      case "reminders":
        return <RemindersModule key={id} />;
      case "focus":
        return <FocusModule key={id} />;
      case "weather":
        return settings?.weatherEnabled ? <WeatherModule key={id} /> : null;
      case "metrics":
        return settings?.metricsEnabled ? <MetricsModule key={id} /> : null;
      case "info":
        return <InfoModule key={id} />;
      case "lyrics":
        return settings?.lyricsEnabled ? <LyricsModule key={id} /> : null;
      case "note":
        return <NoteModule key={id} />;
      case "mic":
        return <MicModule key={id} />;
      case "bluetooth":
        return settings?.bluetoothEnabled ? <BluetoothModule key={id} /> : null;
      case "notifications":
        return <NotificationsModule key={id} />;
      case "clipboard":
        return <ClipboardModule key={id} />;
      default:
        return null;
    }
  };

  /** The active-module content shared by the pill and the strip center. */
  const renderPillContent = (forStrip: boolean) => {
    if (forStrip && pillKind === "clock") return null;
    return (
      <>
        {pillKind === "attention" && (
          <span className={`pill-attention att-${attention?.kind}`}>
            {attention?.kind === "timer-done" && <span>⏰ {attention.label ?? "Timer done"}</span>}
            {attention?.kind === "bt-connected" && <span>🔗 {attention.name}</span>}
            {attention?.kind === "bt-disconnected" && <span>◌ {attention.name}</span>}
            {attention?.kind === "phone-notify" && <span>📱 {attention.title ?? "Phone"}</span>}
            {attention?.kind === "reminder" && <span>🔔 {attention.text}</span>}
            {attention?.kind === "welcome" && <span>{attention.text}</span>}
          </span>
        )}
        {pillKind === "timer" && (
          <span className="pill-timer">
            <span className="pill-dot timer-dot" />
            {fmtCountdown(activeTimer?.remainingMs ?? 0)} {activeTimer?.label}
          </span>
        )}
        {pillKind === "bt" && (
          <span className="pill-timer">
            <span className={`pill-dot ${btFlash?.connected ? "bt-in" : "bt-out"}`} />
            {btFlash?.name} {btFlash?.connected ? "connected" : "disconnected"}
          </span>
        )}
        {(pillKind === "media" || pillKind === "media-dim") && (
          <span className="pill-media">
            {art ? (
              <img className="pill-art" src={art} alt="" />
            ) : (
              <span className="pill-art pill-art-empty">♪</span>
            )}
            <span className="pill-track">
              <span className="pill-title">{media?.title || "Unknown"}</span>
              <span className="pill-artist">{media?.artist}</span>
            </span>
          </span>
        )}
        {pillKind === "notify" && (
          <span className="pill-timer">
            <span className="pill-dot notify-dot" />
            {notifBadge} new notification{notifBadge === 1 ? "" : "s"}
          </span>
        )}
        {pillKind === "clock" && !forStrip && <Clock />}
      </>
    );
  };

  const isStrip = settings?.islandDesign === "strip";
  const now = new Date();
  const dateStr = now.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
  const stripVariant = settings?.spectrumStyle ?? "accent";

  const stripRow = (expanded: boolean) => (
    <div className={`strip ${expanded ? "strip-expanded" : ""}`}>
      <div className="strip-left">
        <span className="strip-brand">Isle</span>
        {!expanded && <span className="strip-date">{dateStr}</span>}
      </div>
      <div className="strip-center">
        {expanded ? (
          media?.title ? (
            <span className="strip-now">{media.title} {media.artist ? `· ${media.artist}` : ""}</span>
          ) : (
            <span className="strip-now muted">panel</span>
          )
        ) : (
          renderPillContent(true)
        )}
      </div>
      <div className="strip-right">
        {pillKind !== "clock" && !expanded && (
          <SpectrumBars bands={10} height={14} variant={stripVariant} tint={tintLive} />
        )}
        <span className="strip-time">
          <MiniTime />
        </span>
        {expanded && (
          <>
            <button
              type="button"
              className="icon-btn"
              aria-label="Open settings"
              onClick={() => api.openSettings()}
            >
              ⚙
            </button>
            <button
              type="button"
              className="icon-btn"
              aria-label="Collapse strip"
              onClick={collapse}
            >
              ⌄
            </button>
          </>
        )}
      </div>
    </div>
  );

  return (
    <IslandContext.Provider value={ctx}>
      <div
        className={`island-root ${mode} theme-${settings?.theme ?? "system"}`}
        style={{ "--media-tint": tintLive ?? "transparent" } as React.CSSProperties}
        onMouseEnter={onPillEnter}
        onMouseLeave={onPillLeave}
        data-expanded={mode === "expanded"}
        data-design={settings?.islandDesign ?? "pill"}
        data-playing={media?.status === "playing" ? "true" : "false"}
      >
        {mode === "compact" && isStrip ? (
          <button
            type="button"
            className="pill strip"
            aria-expanded={false}
            aria-label="Isle strip — click the center to open the panel"
            onClick={expand}
          >
            {stripRow(false)}
          </button>
        ) : mode === "compact" ? (
          <button
            type="button"
            className="pill"
            aria-expanded={false}
            aria-label={`Isle: ${pillKind === "media" ? `${media?.title} — ${media?.artist}` : pillKind === "timer" ? `Timer ${fmtCountdown(activeTimer?.remainingMs ?? 0)}` : "open panel"}`}
            onClick={() => settings?.expandMode === "click" && expand()}
          >
            <div className="pill-left" aria-hidden="true">
              {micMuted && (
                <span className="pill-timer">
                  <span className="pill-dot muted-dot" />
                  mic muted
                </span>
              )}
              {renderPillContent(false)}
            </div>
            <div className="pill-right" aria-hidden="true">
              {pillKind !== "clock" && (
                <SpectrumBars
                  bands={10}
                  height={14}
                  variant={stripVariant}
                  tint={tintLive}
                />
              )}
              {pillKind !== "clock" && (
                <span className="pill-mini-time">
                  <MiniTime />
                </span>
              )}
            </div>
          </button>
        ) : isStrip ? (
          <div className="strip-host">
            {stripRow(true)}
            <div className="panel panel-attached" role="dialog" aria-label="Isle panel" tabIndex={-1}>
              <div className="panel-body" onMouseEnter={onPanelEnter}>
                {settings
                  ? settings.moduleOrder
                      .filter((id) => settings.modulesEnabled[id])
                      .map((id, idx) => (
                        <div
                          key={id}
                          className={`bento-item ${WIDE_MODULES.has(id) ? "wide" : "narrow"}`}
                        style={{ animationDelay: `${idx * 55}ms` }}
                        >
                          {renderModule(id)}
                        </div>
                      ))
                  : null}
              </div>
            </div>
          </div>
        ) : (
          <div className="panel" role="dialog" aria-label="Isle panel" tabIndex={-1}>
            <header className="panel-head">
              <span className="panel-title">Isle</span>
              <div className="panel-head-actions">
                <button
                  type="button"
                  className="icon-btn"
                  aria-label="Open settings"
                  onClick={() => api.openSettings()}
                >
                  ⚙
                </button>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label="Collapse island"
                  onClick={collapse}
                >
                  ⌄
                </button>
              </div>
            </header>
            <div className="panel-body" onMouseEnter={onPanelEnter}>
              {settings
                ? settings.moduleOrder
                    .filter((id) => settings.modulesEnabled[id])
                    .map((id, idx) => (
                      <div
                        key={id}
                        className={`bento-item ${WIDE_MODULES.has(id) ? "wide" : "narrow"}`}
                        style={{ animationDelay: `${idx * 55}ms` }}
                      >
                        {renderModule(id)}
                      </div>
                    ))
                : null}
            </div>
          </div>
        )}
      </div>
    </IslandContext.Provider>
  );
}
