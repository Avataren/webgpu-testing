//! Bounded frame captures, percentile summaries and baseline comparisons.
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn distribution(mut values: Vec<f64>) -> Value {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return Value::Null;
    }
    values.sort_by(f64::total_cmp);
    // Nearest-rank percentiles: a 100-sample p95 is the 95th sorted value.
    let at = |p: f64| values[(values.len() as f64 * p).ceil().max(1.0) as usize - 1];
    json!({"median":at(0.5),"p95":at(0.95),"p99":at(0.99),"max":values[values.len()-1]})
}

pub(crate) fn summarize(samples: &[Value], baseline: Option<&Value>) -> Value {
    let times = samples
        .iter()
        .filter_map(|s| s["frame_time_ms"].as_f64())
        .collect();
    let mut stages: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for sample in samples {
        if let Some(breakdown) = sample["frame_breakdown"].as_object() {
            for (stage, data) in breakdown {
                if let Some(ms) = data["last_ms"].as_f64() {
                    stages.entry(stage.clone()).or_default().push(ms);
                }
            }
        }
    }
    let stages = stages
        .into_iter()
        .map(|(name, values)| (name, distribution(values)))
        .collect::<serde_json::Map<_, _>>();
    // GPU stage times (timestamp queries; a frame or two behind the sample, and the same
    // frame can be read twice, which the percentiles tolerate) and renderer counters.
    let mut gpu_stages: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut gpu_totals = Vec::new();
    for sample in samples {
        let gpu = &sample["renderer"]["gpu"];
        if let Some(total) = gpu["total_ms"].as_f64() {
            gpu_totals.push(total);
        }
        for entry in gpu["stages_ms"].as_array().into_iter().flatten() {
            if let (Some(name), Some(ms)) = (entry[0].as_str(), entry[1].as_f64()) {
                gpu_stages.entry(name.to_owned()).or_default().push(ms);
            }
        }
    }
    let gpu_stages = gpu_stages
        .into_iter()
        .map(|(name, values)| (name, distribution(values)))
        .collect::<serde_json::Map<_, _>>();
    let mut counters = serde_json::Map::new();
    for key in [
        "draw_calls",
        "opaque_draw_calls",
        "shadow_draw_calls",
        "shadow_views_rendered",
        "shadow_views_cached",
        "culled_instances",
        "instances",
        "batches",
    ] {
        let values: Vec<f64> = samples
            .iter()
            .filter_map(|sample| sample["renderer"][key].as_f64())
            .collect();
        if let (Some(min), Some(max), Some(last)) = (
            values.iter().copied().reduce(f64::min),
            values.iter().copied().reduce(f64::max),
            values.last(),
        ) {
            counters.insert(key.into(), json!({"min": min, "max": max, "last": last}));
        }
    }
    let first = samples.first().cloned().unwrap_or(Value::Null);
    let last = samples.last().cloned().unwrap_or(Value::Null);
    let mut growth = serde_json::Map::new();
    for pointer in [
        "/entities",
        "/scripts",
        "/assets/meshes",
        "/assets/materials",
        "/assets/textures",
    ] {
        if let Some((a, b)) = first
            .pointer(pointer)
            .and_then(Value::as_i64)
            .zip(last.pointer(pointer).and_then(Value::as_i64))
        {
            growth.insert(pointer.into(), json!({"first":a,"last":b,"change":b-a}));
        }
    }
    let mut result = json!({"samples":samples.len(),"frame_time_ms":distribution(times),"stages_ms":stages,"growth":growth,
        "gpu_total_ms":distribution(gpu_totals),"gpu_stages_ms":gpu_stages,"renderer_counters":counters,
        "first":first,"last":last,"timing_scope":"stages_ms are CPU frame/stage timings (render_gpu measures submission, not GPU execution; vsync and window waits affect frame time); gpu_stages_ms and gpu_total_ms are GPU milliseconds from timestamp queries, read a frame or two behind each sample; renderer_counters are min/max/last over the samples."});
    let mut slow_scripts = Vec::new();
    if let Some(scripts) = last["script_timings"].as_array() {
        for script in scripts {
            let earlier = first["script_timings"].as_array().and_then(|rows| {
                rows.iter()
                    .find(|r| r["entity"] == script["entity"] && r["source"] == script["source"])
            });
            let cpu = script["cpu_ms"].as_f64().unwrap_or(0.0)
                - earlier.and_then(|r| r["cpu_ms"].as_f64()).unwrap_or(0.0);
            let calls = script["invocations"]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(earlier.and_then(|r| r["invocations"].as_u64()).unwrap_or(0));
            slow_scripts.push(json!({"entity":script["entity"],"source":script["source"],"cpu_ms":cpu.max(0.0),"invocations":calls}));
        }
    }
    slow_scripts.sort_by(|a, b| {
        b["cpu_ms"]
            .as_f64()
            .unwrap()
            .total_cmp(&a["cpu_ms"].as_f64().unwrap())
    });
    result["slow_scripts"] = json!(slow_scripts);
    if let Some(base) = baseline {
        let mut comparison = serde_json::Map::new();
        for statistic in ["median", "p95", "p99"] {
            if let Some((before, after)) = base["frame_time_ms"][statistic]
                .as_f64()
                .zip(result["frame_time_ms"][statistic].as_f64())
            {
                comparison.insert(statistic.into(),json!({"before_ms":before,"after_ms":after,"change_ms":after-before,"change_percent":if before>0.0{Some((after/before-1.0)*100.0)}else{None}}));
            }
        }
        result["comparison"] = Value::Object(comparison);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_tail_latency_growth_and_baseline_changes() {
        let samples=(1..=100).map(|i|json!({"frame_time_ms":i,"entities":i,"assets":{"materials":3},"frame_breakdown":{"scripts":{"last_ms":i as f64/2.0}}})).collect::<Vec<_>>();
        let result = summarize(&samples, Some(&json!({"frame_time_ms":{"p95":50.0}})));
        assert_eq!(result["frame_time_ms"]["p95"], 95.0);
        assert_eq!(result["growth"]["/entities"]["change"], 99);
        assert_eq!(result["comparison"]["p95"]["change_ms"], 45.0);
    }

    #[test]
    fn gpu_stages_and_renderer_counters_are_summarized() {
        let samples = (1..=10)
            .map(|i| {
                json!({
                    "frame_time_ms": 5.0,
                    "renderer": {
                        "draw_calls": 100 + i,
                        "shadow_views_cached": 13,
                        "gpu": {"total_ms": i as f64, "stages_ms": [["main", 0.5], ["shadows", i as f64 / 10.0]]}
                    }
                })
            })
            .collect::<Vec<_>>();
        let result = summarize(&samples, None);
        assert_eq!(result["gpu_total_ms"]["max"], 10.0);
        assert_eq!(result["gpu_stages_ms"]["main"]["median"], 0.5);
        assert_eq!(result["gpu_stages_ms"]["shadows"]["p95"], 1.0);
        assert_eq!(result["renderer_counters"]["draw_calls"]["min"], 101.0);
        assert_eq!(result["renderer_counters"]["draw_calls"]["last"], 110.0);
        assert_eq!(
            result["renderer_counters"]["shadow_views_cached"]["max"],
            13.0
        );
    }
}
