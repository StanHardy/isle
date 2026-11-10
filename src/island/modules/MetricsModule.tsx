import { useEffect, useRef, useState } from "react";
import * as api from "../../shared/api";
import type { MetricSample } from "../../shared/types";
import { useIsland } from "../IslandRoot";

const HISTORY = 40;

function fmtBytes(bps: number): string {
  if (bps < 1024) return `${bps} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
}

function fmtMem(bytes: number): string {
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

export default function MetricsModule() {
  const { settings } = useIsland();
  const [sample, setSample] = useState<MetricSample | null>(null);
  const history = useRef<number[]>([]);

  useEffect(() => {
    api.metricsSubscribe(true).catch(() => {});
    const un = api.listen<MetricSample>(api.ev.METRICS, (e) => {
      history.current = [...history.current, e.payload.cpu].slice(-HISTORY);
      setSample(e.payload);
    });
    return () => {
      un.then((f) => f()).catch(() => {});
      api.metricsSubscribe(false).catch(() => {});
    };
  }, []);

  if (!settings?.metricsEnabled) {
    return (
      <div className="card">
        <h3>System</h3>
        <p className="muted">Metrics are disabled in settings.</p>
      </div>
    );
  }

  const cpu = sample?.cpu ?? 0;
  const memPct = sample && sample.memTotal > 0 ? (sample.memUsed / sample.memTotal) * 100 : 0;

  return (
    <div className="card">
      <h3>System</h3>
      {!sample && <p className="muted">Sampling…</p>}
      {sample && (
        <>
          <div className="metric-row">
            <span className="metric-label">CPU</span>
            <div className="metric-bar" aria-label={`CPU ${cpu.toFixed(0)} percent`}>
              <div className="metric-fill" style={{ width: `${Math.min(100, cpu)}%` }} />
            </div>
            <span className="metric-value">{cpu.toFixed(0)}%</span>
          </div>
          <div className="metric-row">
            <span className="metric-label">RAM</span>
            <div className="metric-bar" aria-label={`Memory ${memPct.toFixed(0)} percent`}>
              <div className="metric-fill" style={{ width: `${memPct}%` }} />
            </div>
            <span className="metric-value">
              {fmtMem(sample.memUsed)} / {fmtMem(sample.memTotal)}
            </span>
          </div>
          <div className="metric-row metric-net">
            <span className="metric-label">Net</span>
            <span className="metric-value net-value">
              ↓ {fmtBytes(sample.netDownBps)} · ↑ {fmtBytes(sample.netUpBps)}
            </span>
          </div>
          <div className="metric-sparks" aria-hidden="true">
            {history.current.map((v, i) => (
              <span
                key={i}
                className="spark"
                style={{ height: `${Math.max(2, Math.min(100, v))}%` }}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}
