use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

#[derive(Deserialize, Clone)]
struct TraceFile {
    #[serde(rename = "traceEvents")]
    trace_events: Vec<TraceEvent>,
}

#[derive(Deserialize, Clone)]
struct TraceEvent {
    name: String,
    /// Duration in microseconds.
    dur: u64,
    /// Timestamp in microseconds (start of event).
    #[serde(default)]
    ts: u64,
}

struct FuncStats {
    total_us: u64,
    max_us: u64,
    count: u64,
    /// Individual durations for percentile computation.
    durations: Vec<u64>,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    // Parse flags.
    let generate_flamegraph = args.iter().any(|a| a == "--flamegraph" || a == "--svg");

    let path = resolve_input_path(&args);

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
            durations: Vec::new(),
        });
        entry.total_us += event.dur;
        if event.dur > entry.max_us {
            entry.max_us = event.dur;
        }
        entry.count += 1;
        entry.durations.push(event.dur);
    }

    // Sort durations for percentile computation.
    for s in stats.values_mut() {
        s.durations.sort_unstable();
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

    // --- Percentile analysis ---
    // (entries still sorted by total time from above)
    println!("=== Percentile Analysis ===");
    println!(
        "  {:<40} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "Name", "P50(ms)", "P95(ms)", "P99(ms)", "Max(ms)", "Calls"
    );
    for (name, s) in entries.iter().take(20) {
        let p50 = percentile(&s.durations, 50.0);
        let p95 = percentile(&s.durations, 95.0);
        let p99 = percentile(&s.durations, 99.0);
        println!(
            "  {:<40} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>8}",
            name,
            p50 as f64 / 1000.0,
            p95 as f64 / 1000.0,
            p99 as f64 / 1000.0,
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

    // --- Flame graph SVG ---
    if generate_flamegraph {
        let svg_path = path.with_extension("svg");
        generate_flamegraph_svg(&trace, &svg_path);
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

/// Compute the p-th percentile from a **sorted** slice (nearest-rank method).
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (p / 100.0 * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn resolve_input_path(args: &[String]) -> PathBuf {
    // First positional argument that is not a flag.
    for arg in args.iter().skip(1) {
        if !arg.starts_with('-') {
            return PathBuf::from(arg);
        }
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

    eprintln!("Usage: gg_tools [OPTIONS] [path-to-profile.json]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --flamegraph, --svg   Generate a flame graph SVG file");
    eprintln!();
    eprintln!("No path given and could not find gg_profile_runtime.json automatically.");
    process::exit(1);
}

// ---------------------------------------------------------------------------
// Flame graph SVG generation
// ---------------------------------------------------------------------------

/// A span on the call stack, derived from a trace event.
struct FlameSpan {
    name: String,
    start_us: u64,
    dur_us: u64,
    depth: usize,
}

/// Build call stack spans from trace events by using timestamps to determine nesting.
fn build_flame_spans(events: &[TraceEvent]) -> Vec<FlameSpan> {
    // Filter to events that have both ts and dur > 0.
    let mut timed: Vec<&TraceEvent> = events.iter().filter(|e| e.dur > 0 && e.ts > 0).collect();

    if timed.is_empty() {
        // Fall back: include events with ts == 0 if they have dur.
        timed = events.iter().filter(|e| e.dur > 0).collect();
    }

    // Sort by start time, then by duration descending (longer spans first — parents before children).
    timed.sort_by(|a, b| a.ts.cmp(&b.ts).then(b.dur.cmp(&a.dur)));

    // Stack of (end_time_us) to track nesting depth.
    let mut stack: Vec<u64> = Vec::new();
    let mut spans = Vec::with_capacity(timed.len());

    for ev in &timed {
        let end = ev.ts + ev.dur;

        // Pop finished spans off the stack.
        while let Some(&top_end) = stack.last() {
            if ev.ts >= top_end {
                stack.pop();
            } else {
                break;
            }
        }

        let depth = stack.len();
        spans.push(FlameSpan {
            name: ev.name.clone(),
            start_us: ev.ts,
            dur_us: ev.dur,
            depth,
        });

        stack.push(end);
    }

    spans
}

/// Generate a warm color from a function name hash.
fn color_from_name(name: &str) -> (u8, u8, u8) {
    let mut hash: u32 = 5381;
    for b in name.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as u32);
    }
    // Warm color palette: reds, oranges, yellows.
    let r = 200 + (hash % 55) as u8;
    let g = 80 + ((hash >> 8) % 120) as u8;
    let b = 30 + ((hash >> 16) % 50) as u8;
    (r, g, b)
}

/// Escape XML special characters.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn generate_flamegraph_svg(trace: &TraceFile, output_path: &Path) {
    let spans = build_flame_spans(&trace.trace_events);
    if spans.is_empty() {
        eprintln!("No timed events found for flame graph generation.");
        return;
    }

    // Determine global time range and max depth.
    let global_start = spans.iter().map(|s| s.start_us).min().unwrap();
    let global_end = spans.iter().map(|s| s.start_us + s.dur_us).max().unwrap();
    let total_dur = global_end - global_start;
    if total_dur == 0 {
        eprintln!("All events have zero duration span; cannot generate flame graph.");
        return;
    }
    let max_depth = spans.iter().map(|s| s.depth).max().unwrap_or(0);

    // SVG parameters.
    let svg_width: f64 = 1200.0;
    let row_height: f64 = 18.0;
    let font_size: f64 = 11.0;
    let title_height: f64 = 30.0;
    let padding_top: f64 = 10.0;
    let svg_height = title_height + padding_top + (max_depth as f64 + 1.0) * row_height + 10.0;

    let mut svg = String::with_capacity(spans.len() * 200);

    // SVG header.
    let title_x = svg_width / 2.0 - 50.0;
    let _ = write!(
        svg,
        r##"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{svg_width}" height="{svg_height}" viewBox="0 0 {svg_width} {svg_height}">
<style>
  text {{ font-family: monospace; font-size: {font_size}px; fill: #000; }}
  .title {{ font-size: 16px; font-weight: bold; fill: #333; }}
  rect.frame:hover {{ stroke: #000; stroke-width: 1; }}
</style>
<rect width="100%" height="100%" fill="#f8f8f8"/>
<text x="{title_x}" y="20" class="title">Flame Graph</text>
"##
    );

    // Render spans — flame graphs grow upward, but for simplicity we render
    // depth 0 at the bottom and deeper calls above.
    let chart_bottom = svg_height - 10.0;

    for span in &spans {
        let x = ((span.start_us - global_start) as f64 / total_dur as f64) * svg_width;
        let w = (span.dur_us as f64 / total_dur as f64) * svg_width;

        // Skip tiny rectangles that would be invisible.
        if w < 0.5 {
            continue;
        }

        let y = chart_bottom - (span.depth as f64 + 1.0) * row_height;

        let (r, g, b) = color_from_name(&span.name);
        let esc_name = xml_escape(&span.name);
        let dur_ms = span.dur_us as f64 / 1000.0;

        let _ = write!(
            svg,
            r#"<rect class="frame" x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{rh:.1}" fill="rgb({r},{g},{b})" rx="1">
<title>{esc_name} ({dur_ms:.3} ms)</title>
</rect>
"#,
            rh = row_height - 1.0
        );

        // Only draw label if the rectangle is wide enough.
        if w > 40.0 {
            // Truncate label to fit.
            let max_chars = (w / (font_size * 0.6)) as usize;
            let label = if esc_name.len() > max_chars && max_chars > 3 {
                format!("{}..", &span.name[..max_chars - 2])
            } else if esc_name.len() > max_chars {
                String::new()
            } else {
                esc_name.clone()
            };
            if !label.is_empty() {
                let text_x = x + 2.0;
                let text_y = y + row_height - 5.0;
                let esc_label = xml_escape(&label);
                let _ = writeln!(
                    svg,
                    r#"<text x="{text_x:.1}" y="{text_y:.1}" clip-path="url(#clip)">{esc_label}</text>"#
                );
            }
        }
    }

    svg.push_str("</svg>\n");

    // Write to file.
    match fs::File::create(output_path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(svg.as_bytes()) {
                eprintln!("Error writing SVG: {e}");
            } else {
                println!();
                println!(
                    "Flame graph written to {} ({} spans)",
                    output_path.display(),
                    spans.len()
                );
            }
        }
        Err(e) => {
            eprintln!("Error creating SVG file {}: {e}", output_path.display());
        }
    }
}
