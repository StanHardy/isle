import { useEffect, useMemo, useState } from "react";
import * as api from "../../shared/api";
import type { BtBattery, BtDevice, ClipEntry, NotifyItem } from "../../shared/types";
import { useIsland } from "../IslandRoot";

function relTime(tsMs: number): string {
  const diff = Date.now() - tsMs;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return new Date(tsMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function ClipboardModule() {
  const { settings, update } = useIsland();
  const [entries, setEntries] = useState<ClipEntry[]>([]);
  const [filter, setFilter] = useState("");
  const [revealed, setRevealed] = useState<Set<number>>(new Set());

  useEffect(() => {
    api.clipboardList().then(setEntries).catch(() => {});
    const un = api.listen<ClipEntry[]>(api.ev.CLIPBOARD_SYNC, (e) => setEntries(e.payload));
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  const visible = useMemo(
    () => entries.filter((e) => e.text.toLowerCase().includes(filter.toLowerCase())),
    [entries, filter],
  );

  if (!settings?.clipboardEnabled) {
    return (
      <div className="card">
        <h3>Clipboard history</h3>
        <p className="muted">
          Off by default. When enabled, text you copy is kept locally in memory only (bounded
          list, nothing leaves this machine). Sensitive-looking entries are masked.
        </p>
        <button type="button" className="chip" onClick={() => update({ clipboardEnabled: true })}>
          Enable clipboard history
        </button>
      </div>
    );
  }

  return (
    <div className="card">
      <h3>Clipboard history <span className="muted">({entries.length})</span></h3>
      <input
        className="card-search"
        type="text"
        placeholder="Search clipboard…"
        aria-label="Search clipboard history"
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />
      {visible.length === 0 && <p className="muted">Nothing here yet.</p>}
      <ul className="clip-list">
        {visible.map((e) => (
          <li key={e.id} className="clip-row" data-sensitive={e.sensitive}>
            <span className="clip-text">
              {e.sensitive && !revealed.has(e.id) ? `${e.text} (masked)` : e.text}
            </span>
            <span className="clip-meta muted">{relTime(e.tsMs)}</span>
            <span className="clip-actions">
              <button
                type="button"
                className="mini-btn"
                aria-label="Copy entry"
                onClick={() => api.clipboardCopy(e.id)}
              >
                ⧉
              </button>
              {e.sensitive && (
                <button
                  type="button"
                  className="mini-btn"
                  aria-label="Reveal entry"
                  onClick={async () => {
                    const full = await api.clipboardReveal(e.id);
                    if (full !== null) {
                      setRevealed((s) => new Set(s).add(e.id));
                    }
                  }}
                >
                  👁
                </button>
              )}
              <button
                type="button"
                className="mini-btn"
                aria-label="Delete entry"
                onClick={async () => {
                  await api.clipboardDelete(e.id);
                  setEntries((list) => list.filter((x) => x.id !== e.id));
                }}
              >
                ✕
              </button>
            </span>
          </li>
        ))}
      </ul>
      <div className="card-actions">
        <button
          type="button"
          className="chip"
          onClick={async () => {
            await api.clipboardClear();
            setEntries([]);
          }}
        >
          Clear all
        </button>
        <button type="button" className="chip" onClick={() => update({ clipboardEnabled: false })}>
          Disable
        </button>
      </div>
    </div>
  );
}

export function NotificationsModule() {
  const { settings, update } = useIsland();
  const [items, setItems] = useState<NotifyItem[]>([]);

  useEffect(() => {
    api.notificationsList().then(setItems).catch(() => {});
    const un = api.listen<NotifyItem>(api.ev.NOTIFY_NEW, (e) =>
      setItems((list) => [e.payload, ...list].slice(0, 50)),
    );
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  if (!settings?.notificationsEnabled) {
    return (
      <div className="card">
        <h3>Notifications</h3>
        <p className="muted">
          Mirrors Windows toast banners while they appear on screen (via UI Automation —
          no package identity needed). Off by default; content stays on this machine.
        </p>
        <button type="button" className="chip" onClick={() => update({ notificationsEnabled: true })}>
          Enable mirroring
        </button>
      </div>
    );
  }

  return (
    <div className="card">
      <h3>Notifications <span className="muted">({items.length})</span></h3>
      {items.length === 0 && <p className="muted">No notifications captured.</p>}
      <ul className="notify-list">
        {items.map((n) => (
          <li key={n.id} className="notify-row">
            <span className="notify-app muted">{n.source === "phone" ? `📱 ${n.app || "Phone"}` : n.app || "Windows"}</span>
            <span className="notify-title">{n.title}</span>
            {n.body && <span className="notify-body">{n.body}</span>}
            <span className="clip-meta muted">{relTime(n.tsMs)}</span>
          </li>
        ))}
      </ul>
      <div className="card-actions">
        <button
          type="button"
          className="chip"
          onClick={async () => {
            await api.notificationsClear();
            setItems([]);
          }}
        >
          Clear
        </button>
        <button type="button" className="chip" onClick={() => update({ notificationsEnabled: false })}>
          Disable
        </button>
      </div>
    </div>
  );
}

export function BluetoothModule() {
  const [devices, setDevices] = useState<BtDevice[]>([]);
  const [batteries, setBatteries] = useState<Map<string, BtBattery>>(new Map());

  useEffect(() => {
    api.btDevices().then(setDevices).catch(() => {});
    const un = api.listen<{ id: string; name: string; connected: boolean }>(
      api.ev.BT_DEVICE,
      (e) => {
        setDevices((list) => {
          const rest = list.filter((d) => d.id !== e.payload.id);
          return [
            { id: e.payload.id, name: e.payload.name, connected: e.payload.connected },
            ...rest,
          ].slice(0, 12);
        });
      },
    );
    const unBat = api.listen<BtBattery>(api.ev.BT_BATTERY, (e) => {
      setBatteries((map) => new Map(map).set(e.payload.id, e.payload));
    });
    return () => {
      un.then((f) => f()).catch(() => {});
      unBat.then((f) => f()).catch(() => {});
    };
  }, []);

  return (
    <div className="card">
      <h3>Audio devices</h3>
      {devices.length === 0 && (
        <p className="muted">No audio endpoints seen yet — connect a device to see events here.</p>
      )}
      <ul className="bt-list">
        {devices.map((d) => (
          <li key={d.id} className="bt-row" data-connected={d.connected}>
            <span className="bt-dot" aria-hidden="true" />
            <span className="bt-name">{d.name}</span>
            {batteries.get(d.id) && (
              <span className="bt-battery">{batteries.get(d.id)!.level}%</span>
            )}
            <span className="bt-state muted">{d.connected ? "connected" : "disconnected"}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
