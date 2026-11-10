import { useEffect, useRef } from "react";
import * as api from "./api";

/**
 * Audio spectrum bars fed by the WASAPI loopback analyzer (audio:spectrum).
 * Updates bar heights imperatively — no React re-renders at ~25 Hz. Fades
 * itself out when nothing is playing. The backend sends 24 bands; a bar
 * count lower than 24 samples them spread across the whole spectrum
 * (otherwise the pill would show only bass).
 */
const SOURCE_BANDS = 24;

export default function SpectrumBars({
  bands = 24,
  height = 34,
  color,
}: {
  bands?: number;
  height?: number;
  color?: string | null;
}) {
  const refs = useRef<(HTMLSpanElement | null)[]>([]);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const visibleRef = useRef(false);

  useEffect(() => {
    const un = api.listen<number[]>(api.ev.SPECTRUM, (e) => {
      const data = e.payload;
      let energy = 0;
      for (let i = 0; i < bands; i++) {
        const el = refs.current[i];
        if (!el) continue;
        const v = data[Math.round((i * (SOURCE_BANDS - 1)) / (bands - 1))] ?? 0;
        energy += v;
        const h = Math.max(2, Math.round(v * height));
        el.style.height = `${h}px`;
        el.style.opacity = v > 0.04 ? "1" : "0.3";
      }
      const on = energy / bands > 0.012;
      if (on !== visibleRef.current && hostRef.current) {
        visibleRef.current = on;
        hostRef.current.style.opacity = on ? "1" : "0";
      }
    });
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, [bands, height]);

  return (
    <div
      ref={hostRef}
      className="spectrum"
      style={{ height, opacity: 0 }}
      aria-hidden="true"
    >
      {Array.from({ length: bands }, (_, i) => (
        <span
          key={i}
          ref={(el) => {
            refs.current[i] = el;
          }}
          className="spec-bar"
          style={color ? { background: color } : undefined}
        />
      ))}
    </div>
  );
}
