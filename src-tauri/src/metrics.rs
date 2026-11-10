use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use sysinfo::Networks;
use sysinfo::System;
use tauri::Manager;

use crate::events::{self, ev};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSample {
    pub cpu: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub net_up_bps: u64,
    pub net_down_bps: u64,
}

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // Prime the CPU sampler: sysinfo needs two spaced reads for a rate.
        let mut sys = System::new();
        let mut nets = Networks::new_with_refreshed_list();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        std::thread::sleep(Duration::from_millis(300));
        loop {
            if enabled(&app) {
                sys.refresh_cpu_usage();
                sys.refresh_memory();
                nets.refresh(true);
                let mut up = 0u64;
                let mut down = 0u64;
                for (_iface, data) in &nets {
                    up += data.transmitted();
                    down += data.received();
                }
                let sample = MetricSample {
                    cpu: sys.global_cpu_usage(),
                    mem_used: sys.used_memory(),
                    mem_total: sys.total_memory(),
                    net_up_bps: up,
                    net_down_bps: down,
                };
                events::emit(&app, ev::METRICS, &sample);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn enabled(app: &tauri::AppHandle) -> bool {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.metrics_enabled || state.metrics_subscribed.load(Ordering::Relaxed)
}
