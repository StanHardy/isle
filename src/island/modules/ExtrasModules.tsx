import { useEffect, useRef, useState } from "react";
import * as api from "../../shared/api";
import type { LyricsPayload, MediaInfo, MediaState } from "../../shared/types";
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

/** TV/movie art + overview for the playing media (TVmaze keyless / optional TMDB). */
export function InfoModule() {
  const media = useMedia();
  const [info, setInfo] = useState<MediaInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const forKey = useRef<string>("");

  useEffect(() => {
    if (!media) {
      setInfo(null);
      forKey.current = "";
      return;
    }
    if (forKey.current === media.artKey) return;
    setInfo(null);
  }, [media?.artKey]);

  const lookup = async () => {
    if (!media) return;
    forKey.current = media.artKey;
    setLoading(true);
    try {
      setInfo(await api.mediaLookup("auto"));
    } finally {
      setLoading(false);
    }
  };

  if (!media?.title) {
    return (
      <div className="card">
        <h3>What you're watching</h3>
        <p className="muted">Play a movie or TV episode to look up art and summaries.</p>
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
            {loading ? "Looking up…" : "Not identified yet. Looking up uses TVmaze (keyless) or your TMDB key."}
          </p>
          <button type="button" className="chip" onClick={lookup} disabled={loading}>
            {loading ? "…" : "Find info"}
          </button>
        </>
      )}
    </div>
  );
}
