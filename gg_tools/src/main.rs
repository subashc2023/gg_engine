use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

#[derive(Deserialize)]
struct TraceFile {
    #[serde(rename = "traceEvents")]
    trace_events: Vec<TraceEvent>,
}

#[derive(Deserialize)]
struct TraceEvent {
    name: String,
    /// Duration in microseconds.
    dur: u64,
}

struct FuncStats {
    total_us: u64,
    max_us: u64,
    count: u64,
}

fn main() {
    let path = resolve_input_path();

    let data = fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {e}", path.display());
        process::exit(1);
    });

    let trace: TraceFile = serde_json::from_str(&data)
        .or_else(|_| {
            // The session was likely killed before end_session() could write the
            // closing "]}". Truncate back to the last complete event and close.
            eprintln!("(repairing truncated JSON)");
            let repaired = repair_truncated_trace(&data);
            serde_json::from_str(&repaired)
        })
        .unwrap_or_else(|e| {
            eprintln!("Error parsing JSON: {e}");
            process::exit(1);
        });

    if trace.trace_events.is_empty() {
        println!("No trace events found.");
        return;
    }

    println!(
        "Loaded {} events from {}\n",
        trace.trace_events.len(),
        path.display()
    );

    // --- Frame summary (detect via "Run loop" events) ---
    let frame_times_us: Vec<u64> = trace
        .trace_events
        .iter()
        .filter(|e| e.name == "Run loop")
        .map(|e| e.dur)
        .collect();

    if !frame_times_us.is_empty() {
        let count = frame_times_us.len();
        let total: u64 = frame_times_us.iter().sum();
        let min = *frame_times_us.iter().min().unwrap();
        let max = *frame_times_us.iter().max().unwrap();
        let avg = total as f64 / count as f64;

        println!("=== Frame Summary ===");
        println!("  Frames:    {count}");
        println!("  Avg frame: {:.3} ms", avg / 1000.0);
        println!("  Min frame: {:.3} ms", min as f64 / 1000.0);
        println!("  Max frame: {:.3} ms", max as f64 / 1000.0);
        println!("  Avg FPS:   {:.1}", 1_000_000.0 / avg);
        println!();
    }

    // --- Aggregate per-function stats ---
    let mut stats: HashMap<&str, FuncStats> = HashMap::new();
    for event in &trace.trace_events {
        let entry = stats.entry(event.name.as_str()).or_insert(FuncStats {
            total_us: 0,
            max_us: 0,
            count: 0,
        });
        entry.total_us += event.dur;
        if event.dur > entry.max_us {
            entry.max_us = event.dur;
        }
        entry.count += 1;
    }

    let mut entries: Vec<(&str, &FuncStats)> = stats.iter().map(|(&k, v)| (k, v)).collect();

    // --- Top functions by total time ---
    entries.sort_by(|a, b| b.1.total_us.cmp(&a.1.total_us));
    println!("=== Top Functions by Total Time ===");
    println!(
        "  {:<40} {:>10} {:>10} {:>10} {:>8}",
        "Name", "Total(ms)", "Avg(ms)", "Max(ms)", "Calls"
    );
    for (name, s) in entries.iter().take(20) {
        let avg_ms = (s.total_us as f64 / s.count as f64) / 1000.0;
        println!(
            "  {:<40} {:>10.3} {:>10.3} {:>10.3} {:>8}",
            name,
            s.total_us as f64 / 1000.0,
            avg_ms,
            s.max_us as f64 / 1000.0,
            s.count,
        );
    }
    println!();

    // --- Top functions by avg time ---
    entries.sort_by(|a, b| {
        let avg_a = a.1.total_us as f64 / a.1.count as f64;
        let avg_b = b.1.total_us as f64 / b.1.count as f64;
        avg_b.partial_cmp(&avg_a).unwrap()
    });
    println!("=== Top Functions by Avg Time ===");
    println!(
        "  {:<40} {:>10} {:>10} {:>8}",
        "Name", "Avg(ms)", "Max(ms)", "Calls"
    );
    for (name, s) in entries.iter().take(20) {
        let avg_ms = (s.total_us as f64 / s.count as f64) / 1000.0;
        println!(
            "  {:<40} {:>10.3} {:>10.3} {:>8}",
            name,
            avg_ms,
            s.max_us as f64 / 1000.0,
            s.count,
        );
    }
    println!();

    // --- Call counts ---
    entries.sort_by(|a, b| b.1.count.cmp(&a.1.count));
    println!("=== Call Counts ===");
    println!("  {:<40} {:>8}", "Name", "Calls");
    for (name, s) in entries.iter().take(20) {
        println!("  {:<40} {:>8}", name, s.count);
    }
}

/// Repair a Chrome Tracing JSON that was truncated mid-write.
///
/// The format is `{"otherData":{},"traceEvents":[{...},{...},{...}]}`.
/// Truncation can happen:
///   - after a complete event + comma  → strip comma, close
///   - mid-event (partial `{...`)       → strip back to last `}`, close
fn repair_truncated_trace(data: &str) -> String {
    // Find the last complete event object by locating the last '}'.
    if let Some(pos) = data.rfind('}') {
        let mut repaired = data[..=pos].to_string();
        repaired.push_str("]}");
        repaired
    } else {
        // No complete events at all — return an empty trace.
        r#"{"otherData":{},"traceEvents":[]}"#.to_string()
    }
}

fn resolve_input_path() -> PathBuf {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        return PathBuf::from(&args[1]);
    }

    // Auto-detect: look next to our own executable.
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("gg_profile_runtime.json");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // Fallback: check CWD.
    let cwd_candidate = Path::new("gg_profile_runtime.json");
    if cwd_candidate.exists() {
        return cwd_candidate.to_path_buf();
    }

    eprintln!("Usage: gg_tools [path-to-profile.json]");
    eprintln!();
    eprintln!("No path given and could not find gg_profile_runtime.json automatically.");
    process::exit(1);
}
