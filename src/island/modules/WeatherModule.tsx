import { useCallback, useEffect, useState } from "react";
import * as api from "../../shared/api";
import type { GeoResult, WeatherData } from "../../shared/types";
import { useIsland } from "../IslandRoot";

/** WMO code → short description (mirrors describe_code in Rust). */
function describe(code: number): string {
  const table: Record<number, string> = {
    0: "Clear sky", 1: "Mainly clear", 2: "Partly cloudy", 3: "Overcast",
    45: "Fog", 48: "Fog", 51: "Drizzle", 53: "Drizzle", 55: "Drizzle",
    56: "Freezing drizzle", 57: "Freezing drizzle",
    61: "Rain", 63: "Rain", 65: "Rain", 66: "Freezing rain", 67: "Freezing rain",
    71: "Snow", 73: "Snow", 75: "Snow", 77: "Snow grains",
    80: "Rain showers", 81: "Rain showers", 82: "Rain showers",
    85: "Snow showers", 86: "Snow showers",
    95: "Thunderstorm", 96: "Thunderstorm", 99: "Thunderstorm",
  };
  return table[code] ?? "Unknown";
}

function weatherIcon(data: WeatherData): string {
  const c = data.code;
  if (c === 0 || c === 1) return data.isDay ? "☀️" : "🌙";
  if (c === 2) return "⛅";
  if (c === 3) return "☁️";
  if (c === 45 || c === 48) return "🌫️";
  if (c >= 51 && c <= 67) return "🌦️";
  if (c >= 71 && c <= 86) return "🌨️";
  if (c >= 95) return "⛈️";
  return "🌡️";
}

export default function WeatherModule() {
  const { settings } = useIsland();
  const [weather, setWeather] = useState<WeatherData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<GeoResult[] | null>(null);
  const [searching, setSearching] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const data = await api.weatherRefresh();
      if ("error" in data) {
        setError(data.error);
        setWeather(null);
      } else {
        setWeather(data);
        setError(null);
      }
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, settings?.weatherCity?.name]);

  const search = async () => {
    if (!query.trim()) return;
    setSearching(true);
    try {
      setResults(await api.weatherGeocode(query));
    } catch (e) {
      setError(String(e));
      setResults(null);
    } finally {
      setSearching(false);
    }
  };

  const tempUnit = settings?.tempUnit ?? "metric";
  const windUnit = settings?.windUnit ?? "kmh";
  const deg = (t: number) => `${Math.round(t)}°${tempUnit === "imperial" ? "F" : "C"}`;
  const windLabel = windUnit === "mph" ? " mph" : " km/h";
  const city = settings?.weatherCity?.name;

  return (
    <div className="card">
      <h3>Weather</h3>
      {!city && !weather && (
        <p className="muted">Pick a city to see current conditions (Open-Meteo, keyless).</p>
      )}
      {city && !weather && !error && <p className="muted">Loading…</p>}
      {error && <p className="error-text">Weather unavailable: {error}</p>}
      {weather && (
        <>
        <div className="weather-now">
          <span className="weather-icon" aria-hidden="true">{weatherIcon(weather)}</span>
          <div className="weather-main">
            <span className="weather-temp">{deg(weather.temp)}</span>
            <span className="weather-desc">{describe(weather.code)} · {weather.city}</span>
            <span className="weather-extra muted">
              {deg(weather.hi)} / {deg(weather.lo)} · wind {Math.round(weather.wind)}{windLabel} · rain {weather.precipProb}%
            </span>
          </div>
        </div>
        {weather.hourly && weather.hourly.length > 0 && (
          <div className="hourly-row" aria-label="Next hours forecast">
            {weather.hourly.map((h) => (
              <div key={h.time} className="hour-slot" title={`${describe(h.code)} · ${h.precip}% rain`}>
                <span className="hour-time">{h.time.slice(0, 5)}</span>
                <span className="hour-temp">{Math.round(h.temp)}°</span>
                <span className="hour-precip">{h.precip}%</span>
              </div>
            ))}
          </div>
        )}
        </>
      )}
      <form
        className="weather-search"
        onSubmit={(e) => {
          e.preventDefault();
          search();
        }}
      >
        <input
          type="text"
          placeholder="Search city…"
          aria-label="Search city"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button type="submit" className="chip" disabled={searching}>
          {searching ? "…" : "Find"}
        </button>
      </form>
      {results && (
        <ul className="geo-results">
          {results.length === 0 && <li className="muted">No matches.</li>}
          {results.map((r) => (
            <li key={`${r.lat},${r.lon}`}>
              <button
                type="button"
                className="geo-row"
                onClick={async () => {
                  await api.weatherSetCity(r);
                  setResults(null);
                  setQuery("");
                  refresh();
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
    </div>
  );
}
