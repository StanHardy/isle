import { useEffect, useRef, useState } from "react";
import * as api from "../../shared/api";
import { useIsland } from "../IslandRoot";

const PRESETS = [
  { label: "5 min", seconds: 300 },
  { label: "10 min", seconds: 600 },
  { label: "25 min", seconds: 1500 },
  { label: "1 hour", seconds: 3600 },
];

function fmt(ms: number) {
  const s = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}`
    : `${m}:${String(r).padStart(2, "0")}`;
}

function TimerRow({ id, label, totalMs, remainingMs, paused }: {
  id: number; label: string; totalMs: number; remainingMs: number; paused: boolean;
}) {
  const pct = totalMs > 0 ? Math.max(0, Math.min(100, (remainingMs / totalMs) * 100)) : 0;
  return (
    <div className="timer-row" data-paused={paused}>
      <div className="timer-info">
        <span className="timer-label">{label}</span>
        <span className="timer-time">{fmt(remainingMs)}</span>
      </div>
      <div className="timer-bar">
        <div className="timer-fill" style={{ width: `${pct}%` }} />
      </div>
      <div className="timer-actions">
        {paused ? (
          <button type="button" className="mini-btn" aria-label="Resume timer" onClick={() => api.timerResume(id)}>▶</button>
        ) : (
          <button type="button" className="mini-btn" aria-label="Pause timer" onClick={() => api.timerPause(id)}>⏸</button>
        )}
        <button type="button" className="mini-btn" aria-label="Cancel timer" onClick={() => api.timerCancel(id)}>✕</button>
      </div>
    </div>
  );
}

export default function TimersModule() {
  const { timers } = useIsland();
  const [label, setLabel] = useState("");
  const [minutes, setMinutes] = useState("10");
  const inputRef = useRef<HTMLInputElement>(null);

  const add = async (l: string, seconds: number) => {
    await api.timerAdd(l || "Timer", seconds);
  };

  return (
    <div className="card">
      <h3>Timers</h3>
      {timers.length === 0 && <p className="muted">No active timers.</p>}
      <div className="timer-list">
        {timers.map((t) => (
          <TimerRow key={t.id} {...t} />
        ))}
      </div>
      <div className="timer-presets" role="group" aria-label="Timer presets">
        {PRESETS.map((p) => (
          <button key={p.label} type="button" className="chip" onClick={() => add(p.label, p.seconds)}>
            {p.label}
          </button>
        ))}
      </div>
      <form
        className="timer-form"
        onSubmit={(e) => {
          e.preventDefault();
          const secs = Math.round(Number(minutes) * 60);
          if (secs > 0) add(label, secs);
          setLabel("");
        }}
      >
        <input
          ref={inputRef}
          type="text"
          placeholder="Label (optional)"
          aria-label="Timer label"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
        />
        <input
          type="number"
          min="0.1"
          step="0.5"
          aria-label="Timer minutes"
          value={minutes}
          onChange={(e) => setMinutes(e.target.value)}
        />
        <button type="submit" className="chip">Start</button>
      </form>
    </div>
  );
}
