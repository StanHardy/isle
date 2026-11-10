use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::{CityRef, SpeedUnit, TempUnit};
use crate::events::{self, ev};

const GEO_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Isle/1.0 (local-first island)")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client builds")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoResult {
    pub name: String,
    pub admin1: Option<String>,
    pub country: Option<String>,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherData {
    pub city: String,
    pub temp: f32,
    pub code: u32,
    pub wind: f32,
    pub humidity: u32,
    pub hi: f32,
    pub lo: f32,
    pub precip_prob: u32,
    pub is_day: bool,
    pub fetched_ms: u64,
}

#[derive(Deserialize)]
struct GeoResponse {
    results: Option<Vec<GeoRaw>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeoRaw {
    name: String,
    admin1: Option<String>,
    country: Option<String>,
    latitude: f64,
    longitude: f64,
}

#[derive(Deserialize)]
struct ForecastResponse {
    current: CurrentBlock,
    daily: DailyBlock,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct CurrentBlock {
    temperature_2m: f32,
    weather_code: u32,
    wind_speed_10m: f32,
    relative_humidity_2m: u32,
    is_day: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct DailyBlock {
    temperature_2m_max: Vec<f32>,
    temperature_2m_min: Vec<f32>,
    precipitation_probability_max: Vec<u32>,
}

pub async fn geocode(query: &str) -> Result<Vec<GeoResult>, String> {
    let resp = client()
        .get(GEO_URL)
        .query(&[
            ("name", query),
            ("count", "6"),
            ("language", "en"),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<GeoResponse>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp
        .results
        .unwrap_or_default()
        .into_iter()
        .map(|r| GeoResult {
            name: r.name,
            admin1: r.admin1,
            country: r.country,
            lat: r.latitude,
            lon: r.longitude,
        })
        .collect())
}

pub async fn fetch(
    city: &CityRef,
    temp_unit: &TempUnit,
    wind_unit: &SpeedUnit,
) -> Result<WeatherData, String> {
    let temp_unit = match temp_unit {
        TempUnit::Imperial => "fahrenheit",
        TempUnit::Metric => "celsius",
    };
    let wind_unit = match wind_unit {
        SpeedUnit::Mph => "mph",
        SpeedUnit::Kmh => "kmh",
    };
    let resp = client()
        .get(FORECAST_URL)
        .query(&[
            ("latitude".to_string(), format!("{}", city.lat)),
            ("longitude".to_string(), format!("{}", city.lon)),
            (
                "current".to_string(),
                "temperature_2m,weather_code,wind_speed_10m,relative_humidity_2m,is_day".to_string(),
            ),
            (
                "daily".to_string(),
                "temperature_2m_max,temperature_2m_min,precipitation_probability_max".to_string(),
            ),
            ("forecast_days".to_string(), "1".to_string()),
            ("timezone".to_string(), "auto".to_string()),
            ("temperature_unit".to_string(), temp_unit.to_string()),
            ("wind_speed_unit".to_string(), wind_unit.to_string()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<ForecastResponse>()
        .await
        .map_err(|e| e.to_string())?;

    Ok(WeatherData {
        city: city.name.clone(),
        temp: resp.current.temperature_2m,
        code: resp.current.weather_code,
        wind: resp.current.wind_speed_10m,
        humidity: resp.current.relative_humidity_2m,
        hi: resp.daily.temperature_2m_max.first().copied().unwrap_or(0.0),
        lo: resp.daily.temperature_2m_min.first().copied().unwrap_or(0.0),
        precip_prob: resp
            .daily
            .precipitation_probability_max
            .first()
            .copied()
            .unwrap_or(0),
        is_day: resp.current.is_day != 0,
        fetched_ms: crate::clipboard::now_ms(),
    })
}

/// Weather refresh service: 10-minute poll when enabled with a chosen city.
pub fn start_service(app: tauri::AppHandle) {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        let mut last = Instant::now() - Duration::from_secs(600);
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let (enabled, city, temp_unit, wind_unit) = {
                let state: tauri::State<crate::state::AppState> = app.state();
                let s = state.settings.read().expect("settings lock");
                (
                    s.weather_enabled,
                    s.weather_city.clone(),
                    s.temp_unit,
                    s.wind_unit,
                )
            };
            if !(enabled && city.is_some()) {
                continue;
            }
            if last.elapsed() < Duration::from_secs(600) {
                continue;
            }
            last = Instant::now();
            let city = city.expect("city checked above");
            match fetch(&city, &temp_unit, &wind_unit).await {
                Ok(data) => events::emit(&app, ev::WEATHER, &data),
                Err(err) => events::emit(
                    &app,
                    ev::WEATHER,
                    &serde_json::json!({ "error": err, "city": city.name }),
                ),
            }
        }
    });
}

/// Human text for WMO weather codes. The FE WeatherModule mirrors this
/// table; kept here as the tested source of truth.
#[allow(dead_code)]
pub fn describe_code(code: u32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow grains",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm, hail",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_descriptions_cover_common_codes() {
        assert_eq!(describe_code(0), "Clear sky");
        assert_eq!(describe_code(3), "Overcast");
        assert_eq!(describe_code(63), "Rain");
        assert_eq!(describe_code(95), "Thunderstorm");
        assert_eq!(describe_code(9999), "Unknown");
    }
}
