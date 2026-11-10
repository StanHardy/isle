import { useEffect, useRef, useState } from "react";
import * as api from "../../shared/api";
import SpectrumBars from "../../shared/SpectrumBars";
import { useIsland } from "../IslandRoot";

/**
 * Now-playing card: artwork, track info, locally-advancing progress,
 * spectrum bars (WASAPI loopback) and transport controls (SMTC exposes no
 * seek API, so there is no scrubber).
 */
export default function MediaModule({ art, tint }: { art: string | null; tint: string | null }) {
  const { media } = useIsland();
  const [pos, setPos] = useState(0);
  const baseRef = useRef<{ ms: number; at: number; playing: boolean }>({
    ms: 0,
    at: 0,
    playing: false,
  });

  useEffect(() => {
    if (!media) return;
    baseRef.current = {
      ms: media.positionMs,
      at: Date.now(),
      playing: media.status === "playing",
    };
    setPos(media.positionMs);
  }, [media?.artKey, media?.status]);

  useEffect(() => {
    const t = setInterval(() => {
      const b = baseRef.current;
      if (b.playing) setPos(b.ms + (Date.now() - b.at));
    }, 500);
    return () => clearInterval(t);
  }, []);

  if (!media || !media.title) {
    return (
      <div className="card">
        <h3>Now playing</h3>
        <p className="muted">No media session — play something to see it here.</p>
      </div>
    );
  }

  const playing = media.status === "playing";
  const shown = media.durationMs > 0 ? Math.min(pos, media.durationMs) : pos;
  const pct = media.durationMs > 0 ? Math.min(100, (pos / media.durationMs) * 100) : 0;
  const fmt = (ms: number) => {
    const s = Math.floor(ms / 1000);
    const m = Math.floor(s / 60);
    return `${m}:${String(s % 60).padStart(2, "0")}`;
  };

  return (
    <div className="card media-card" data-status={media.status}>
      <h3>Now playing</h3>
      <div className="media-row">
        {art ? (
          <img className="media-art" src={art} alt="" />
        ) : (
          <div className="media-art media-art-empty" aria-hidden="true">♪</div>
        )}
        <div className="media-meta">
          <div className="media-title" title={media.title}>{media.title}</div>
          <div className="media-artist" title={`${media.artist} · ${media.app}`}>{media.artist}</div>
          {media.album && <div className="media-album">{media.album}</div>}
        </div>
      </div>
      <div className="media-progress" role="timer" aria-label="Playback progress">
        <span className="progress-time">{fmt(shown)}</span>
        <div className="progress-bar">
          <div className="progress-fill" style={{ width: `${pct}%` }} />
        </div>
        <span className="progress-time">{fmt(media.durationMs)}</span>
      </div>
      <SpectrumBars bands={24} height={36} color={tint} />
      <div className="media-controls">
        <button
          type="button"
          className="ctrl-btn"
          aria-label="Previous track"
          disabled={!media.canPrev}
          onClick={() => api.mediaPrev()}
        >
          ⏮
        </button>
        <button
          type="button"
          className="ctrl-btn ctrl-main"
          aria-label={playing ? "Pause" : "Play"}
          disabled={playing ? !media.canPause : !media.canPlay}
          onClick={() => api.mediaPlayPause()}
        >
          {playing ? "⏸" : "▶"}
        </button>
        <button
          type="button"
          className="ctrl-btn"
          aria-label="Next track"
          disabled={!media.canNext}
          onClick={() => api.mediaNext()}
        >
          ⏭
        </button>
      </div>
      <p className="media-app muted">via {media.app}</p>
    </div>
  );
}
