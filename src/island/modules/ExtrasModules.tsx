import { useEffect, useRef, useState } from "react";
import * as api from "../../shared/api";
import type { LyricsPayload, MediaInfo, MediaState, Reminder } from "../../shared/types";
import { useIsland } from "../IslandRoot";

function useMedia(): MediaState | null {
  const { media } = useIsland();
  return media;
}

/** Locally advancing playback position (ms). */
function usePosition(media: MediaState | null): number {
  const [pos, setPos] = useState(0);
  const base = useRef<{ ms: number; at: number; playing: boolean }>({
    ms: 0,
    at: 0,
    playing: false,
  });
  useEffect(() => {
    if (!media) return;
    base.current = { ms: media.positionMs, at: Date.now(), playing: media.status === "playing" };
    setPos(media.positionMs);
  }, [media?.artKey, media?.status]);
  useEffect(() => {
    const t = setInterval(() => {
      const b = base.current;
      if (b.playing) setPos(b.ms + (Date.now() - b.at));
    }, 250);
    return () => clearInterval(t);
  }, []);
  return pos;
}

export function LyricsModule() {
  const media = useMedia();
  const pos = usePosition(media);
  const [payload, setPayload] = useState<LyricsPayload | null>(null);
  const [state, setState] = useState<"idle" | "loading" | "miss">("idle");
  const [plainMode, setPlainMode] = useState(false);
  const forKey = useRef<string>("");
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    if (!media || !media.title) {
      setPayload(null);
      setState("idle");
      return;
    }
    if (forKey.current === media.artKey) return;
    forKey.current = media.artKey;
    setState("loading");
    api
      .lyricsFetch(media.artist, media.title, Math.round(media.durationMs / 1000), media.album)
      .then((p) => {
        setPayload(p);
        setState(p && (p.synced.length > 0 || p.plain) ? "idle" : "miss");
      })
      .catch(() => setState("miss"));
  }, [media?.artKey, media?.title]);

  const activeIdx = payload?.synced
    ? payload.synced.reduce((acc, line, i) => (line.tMs <= pos + 250 ? i : acc), -1)
    : -1;

  useEffect(() => {
    if (plainMode || !listRef.current || activeIdx < 0) return;
    const el = listRef.current.querySelector<HTMLElement>(`.lyric-line.active`);
    el?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [activeIdx, plainMode]);

  if (!media?.title) {
    return (
      <div className="card">
        <h3>Lyrics</h3>
        <p className="muted">Play something to follow along (LRCLIB, keyless).</p>
      </div>
    );
  }

  return (
    <div className="card lyrics-card">
      <h3>
        Lyrics
        {payload && payload.synced.length > 0 && (
          <button
            type="button"
            className="mini-btn"
            onClick={() => setPlainMode((m) => !m)}
            aria-label="Toggle plain lyrics"
          >
            {plainMode ? "synced" : "full"}
          </button>
        )}
      </h3>
      {state === "loading" && <p className="muted">Looking up lyrics…</p>}
      {state === "miss" && <p className="muted">No lyrics found for this track.</p>}
      {payload && payload.synced.length > 0 && !plainMode && (
        <ul className="lyrics-list" ref={listRef}>
          {payload.synced.map((line, i) => (
            <li
              key={`${line.tMs}-${i}`}
              className={`lyric-line ${i === activeIdx ? "active" : ""} ${i < activeIdx ? "past" : ""}`}
            >
              {line.text}
            </li>
          ))}
        </ul>
      )}
      {payload?.plain && (plainMode || payload.synced.length === 0) && (
        <pre className="lyrics-plain">{payload.plain}</pre>
      )}
      {payload && <p className="muted provider-note">via {payload.provider}</p>}
    </div>
  );
}

/**
 * TV/movie art + overview. The backend auto-looks-up video-looking media
 * (episode/year titles) in the background and pushes media:info; music is
 * left alone unless the user clicks Find info manually.
 */
function looksLikeVideo(rawTitle: string): boolean {
  if (/(?:^|[^a-z\d])s\d{1,2}e\d{1,3}(?:[^a-z\d]|$)/i.test(rawTitle)) return true;
  return /(?:^|\D)(19\d\d|20\d\d)(?=\D|$)/.test(rawTitle);
}

export function InfoModule() {
  const media = useMedia();
  const [info, setInfo] = useState<MediaInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const forKey = useRef<string>("");
  const cache = useRef(new Map<string, MediaInfo>());

  // Reset per track; serve from the local cache or the backend cache.
  useEffect(() => {
    if (!media) {
      setInfo(null);
      forKey.current = "";
      return;
    }
    if (forKey.current === media.artKey) return;
    forKey.current = media.artKey;
    const local = cache.current.get(media.artKey);
    if (local) {
      setInfo(local);
      return;
    }
    setInfo(null);
    api
      .mediaInfoCached()
      .then((cached) => {
        if (forKey.current !== media.artKey) return;
        if (cached) {
          cache.current.set(media.artKey, cached);
          setInfo(cached);
        } else if (looksLikeVideo(media.rawTitle)) {
          setLoading(true); // backend auto-lookup is on its way
        }
      })
      .catch(() => {});
  }, [media?.artKey]);

  // Live results from the backend auto-lookup.
  useEffect(() => {
    const un = api.listen<{ key: string; info: MediaInfo }>(api.ev.MEDIA_INFO, (e) => {
      cache.current.set(e.payload.key, e.payload.info);
      if (media && e.payload.key === media.artKey) {
        setInfo(e.payload.info);
        setLoading(false);
      }
    });
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, [media?.artKey]);

  const lookup = async () => {
    if (!media) return;
    forKey.current = media.artKey;
    setLoading(true);
    try {
      const result = await api.mediaLookup("auto");
      if (forKey.current !== media.artKey) return;
      if (result) {
        cache.current.set(media.artKey, result);
        setInfo(result);
      }
    } finally {
      setLoading(false);
    }
  };

  if (!media?.title) {
    return (
      <div className="card">
        <h3>What you're watching</h3>
        <p className="muted">Play a movie or TV episode to get art and summaries automatically.</p>
      </div>
    );
  }

  return (
    <div className="card info-card">
      <h3>What you're watching</h3>
      {info ? (
        <div className="info-body">
          {info.poster && <img className="info-poster" src={info.poster} alt="" />}
          <div className="info-text">
            <div className="info-title">{info.title}</div>
            <p className="info-overview">{info.overview || "No summary available."}</p>
            <p className="muted">via {info.provider}</p>
          </div>
        </div>
      ) : (
        <>
          <p className="muted">
            {loading
              ? "Looking up…"
              : "Not identified yet. Episodes and movies are looked up automatically; you can also search manually."}
          </p>
          <button type="button" className="chip" onClick={lookup} disabled={loading}>
            {loading ? "…" : "Find info"}
          </button>
        </>
      )}
    </div>
  );
}


/** Persistent one-note scratchpad (saved to disk on idle). */
export function NoteModule() {
  const [text, setText] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    api.noteGet().then(setText).catch(() => setText(""));
  }, []);

  const onChange = (v: string) => {
    setText(v);
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(async () => {
      await api.noteSave(v);
      setSaved(true);
      window.setTimeout(() => setSaved(false), 1200);
    }, 500);
  };

  return (
    <div className="card">
      <h3>
        Note
        {saved && <span className="muted">saved</span>}
      </h3>
      {text === null ? (
        <p className="muted">Loading…</p>
      ) : (
        <textarea
          className="note-area"
          value={text}
          placeholder="Jot something down — saved automatically."
          onChange={(e) => onChange(e.target.value)}
        />
      )}
    </div>
  );
}


/** Focus minutes per day (every completed timer credits its minutes). */
export function FocusModule() {
  const [days, setDays] = useState<[string, number][]>([]);

  const load = () => {
    api.focusStats().then((d) => setDays(d ?? [])).catch(() => {});
  };

  useEffect(() => {
    load();
    const un = api.listen<unknown>(api.ev.TIMER_DONE, () => load());
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  const max = Math.max(1, ...days.map(([, m]) => m));
  const today = days.length ? days[days.length - 1] : ["", 0];

  return (
    <div className="card">
      <h3>Focus</h3>
      <div className="focus-total">
        <span className="focus-minutes">{today[1]}</span>
        <span className="focus-unit"> min focused today</span>
      </div>
      <div className="focus-bars">
        {days.map(([day, m]) => (
          <div key={day} className="focus-col" title={`${day}: ${m} min`}>
            <div className="focus-bar" style={{ height: `${Math.round((m / max) * 36) + 2}px` }} />
            <span className="focus-day">{day.slice(8)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}


/** Repeat reminders: recurring nudges that fire a banner + chime. */
export function RemindersModule() {
  const [list, setList] = useState<Reminder[]>([]);
  const [text, setText] = useState("");
  const [intervalMin, setIntervalMin] = useState(30);

  useEffect(() => {
    api.remindersList().then(setList).catch(() => {});
    const un = api.listen<Reminder[]>(api.ev.REMINDERS_SYNC, (e) => setList(e.payload));
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  return (
    <div className="card">
      <h3>Reminders</h3>
      {list.length === 0 && <p className="muted">No repeating reminders.</p>}
      <ul className="reminders-list">
        {list.map((r) => (
          <li key={r.id} className="reminder-row">
            <span className="reminder-text">{r.text}</span>
            <span className="clip-meta muted">every {r.intervalMin}m</span>
            <button
              type="button"
              className="mini-btn"
              aria-label="Delete reminder"
              onClick={async () => {
                await api.reminderDelete(r.id);
                setList((l) => l.filter((x) => x.id !== r.id));
              }}
            >
              ✕
            </button>
          </li>
        ))}
      </ul>
      <form
        className="timer-form"
        onSubmit={async (e) => {
          e.preventDefault();
          if (!text.trim()) return;
          await api.reminderAdd(text.trim(), intervalMin);
          setText("");
        }}
      >
        <input
          type="text"
          placeholder="Remind me to…"
          aria-label="Reminder text"
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <select
          aria-label="Reminder interval"
          value={intervalMin}
          onChange={(e) => setIntervalMin(Number(e.target.value))}
        >
          <option value="15">15m</option>
          <option value="30">30m</option>
          <option value="45">45m</option>
          <option value="60">60m</option>
        </select>
        <button type="submit" className="chip">Add</button>
      </form>
    </div>
  );
}
