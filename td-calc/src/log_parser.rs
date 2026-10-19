//! Parser and replay helpers for `logged_data.txt` (captured by `data_logger.rs`).

use std::collections::BTreeMap;
use std::format;
use std::string::{String, ToString};
use std::vec::Vec;

use crate::optics::{ChannelBaselines, DualReading, SpectralReadings};

/// A single raw CSV line: `D,<sweep>,<millis>,<led>,<r>,<g>,<b>,<sample>,<als_raw>,<white_raw>`.
#[derive(Debug, Clone)]
pub struct RawSampleRow {
    pub sweep: u32,
    pub millis: u64,
    pub led: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub sample: usize,
    pub als_raw: u32,
    pub white_raw: u32,
}

/// Ground-truth annotations parsed from `# INSERTED: <name> (TD: <td_gt>, Hex: #<hex_gt>)`.
#[derive(Debug, Clone)]
pub struct FilamentGroundTruth {
    pub name: String,
    pub raw_header: String,
    pub expected_td: Option<f32>,
    pub expected_rgb: Option<(u8, u8, u8)>,
    pub expected_hex: Option<String>,
}

/// One full illumination sweep for a filament insertion.
#[derive(Debug, Clone)]
pub struct FilamentSweep {
    pub sweep_id: u32,
    pub rows: Vec<RawSampleRow>,
}

/// A recorded filament insertion session (typically containing 2 sweeps).
#[derive(Debug, Clone)]
pub struct FilamentSession {
    pub ground_truth: FilamentGroundTruth,
    pub baselines: ChannelBaselines,
    pub sweeps: Vec<FilamentSweep>,
}

/// A pre-insertion detection cycle (`off` followed by `detect`).
#[derive(Debug, Clone)]
pub struct DetectionCycleRecord {
    pub sweep: u32,
    pub off_als: f32,
    pub off_white: f32,
    pub detect_level: u8,
    pub detect_als: f32,
    pub detect_white: f32,
}

/// Parsed representation of a `logged_data.txt` capture file.
#[derive(Debug, Clone)]
pub struct LoggedDataset {
    pub baselines: ChannelBaselines,
    pub detection_cycles: Vec<DetectionCycleRecord>,
    pub sessions: Vec<FilamentSession>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleAggregation {
    /// Median of the first 3 samples (`0..=2`), mirroring `v77::sample_median(veml, 3)`.
    First3Median,
    /// Median of all logged samples (`0..=4`).
    AllMedian,
    /// Median of settled samples (`1..`), skipping sample 0 which can straddle the LED transition.
    SettledMedian,
}

pub fn median_f32(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n.is_multiple_of(2) {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    } else {
        values[n / 2]
    }
}

impl FilamentSweep {
    /// Compute the median `(als, white)` reading for a given state label (e.g. `"off"`, `"r255"`).
    pub fn state_reading(&self, state: &str, agg: SampleAggregation) -> Option<DualReading> {
        let filtered: Vec<&RawSampleRow> = self
            .rows
            .iter()
            .filter(|r| {
                r.led == state
                    && match agg {
                        SampleAggregation::First3Median => r.sample <= 2,
                        SampleAggregation::AllMedian => true,
                        SampleAggregation::SettledMedian => r.sample >= 1,
                    }
            })
            .collect();

        if filtered.is_empty() {
            return None;
        }

        let mut als_vals: Vec<f32> = filtered.iter().map(|r| r.als_raw as f32).collect();
        let mut white_vals: Vec<f32> = filtered.iter().map(|r| r.white_raw as f32).collect();

        Some(DualReading::new(
            median_f32(&mut als_vals),
            median_f32(&mut white_vals),
        ))
    }

    /// Extract ambient-compensated `SpectralReadings` at a specific LED level (`255`, `192`, `128`, `64`, `32`).
    pub fn spectral_readings_at_level(
        &self,
        level: u8,
        agg: SampleAggregation,
    ) -> Option<SpectralReadings> {
        let ambient = self.state_reading("off", agg)?;
        let r_state = format!("r{}", level);
        let g_state = format!("g{}", level);
        let b_state = format!("b{}", level);

        let r = self.state_reading(&r_state, agg)?.net(ambient);
        let g = self.state_reading(&g_state, agg)?.net(ambient);
        let b = self.state_reading(&b_state, agg)?.net(ambient);

        Some(SpectralReadings {
            red: r,
            green: g,
            blue: b,
            red_level: level,
            green_level: level,
            blue_level: level,
        })
    }
}

impl FilamentSession {
    /// Combine all sweeps in the session using median aggregation (mirroring `MEASUREMENT_CYCLES = 2` in firmware).
    pub fn combined_spectral_readings(
        &self,
        level: u8,
        agg: SampleAggregation,
    ) -> Option<SpectralReadings> {
        let mut r_als = Vec::new();
        let mut r_white = Vec::new();
        let mut g_als = Vec::new();
        let mut g_white = Vec::new();
        let mut b_als = Vec::new();
        let mut b_white = Vec::new();

        for sweep in &self.sweeps {
            if let Some(sr) = sweep.spectral_readings_at_level(level, agg) {
                r_als.push(sr.red.als);
                r_white.push(sr.red.white);
                g_als.push(sr.green.als);
                g_white.push(sr.green.white);
                b_als.push(sr.blue.als);
                b_white.push(sr.blue.white);
            }
        }

        if r_white.is_empty() {
            return None;
        }

        Some(SpectralReadings {
            red: DualReading::new(median_f32(&mut r_als), median_f32(&mut r_white)),
            green: DualReading::new(median_f32(&mut g_als), median_f32(&mut g_white)),
            blue: DualReading::new(median_f32(&mut b_als), median_f32(&mut b_white)),
            red_level: level,
            green_level: level,
            blue_level: level,
        })
    }
}

pub fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let clean = hex.trim().trim_start_matches('#');
    if clean.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
    Some((r, gt_g(g), b))
}

#[inline]
fn gt_g(g: u8) -> u8 {
    g
}

fn parse_inserted_header(line: &str) -> FilamentGroundTruth {
    let raw = line
        .trim_start_matches("# INSERTED")
        .trim_start_matches(':')
        .trim()
        .to_string();

    if raw.is_empty() {
        return FilamentGroundTruth {
            name: "Unknown".to_string(),
            raw_header: line.to_string(),
            expected_td: None,
            expected_rgb: None,
            expected_hex: None,
        };
    }

    let name = raw
        .split('(')
        .next()
        .unwrap_or(&raw)
        .trim()
        .to_string();

    let mut expected_td = None;
    let mut expected_hex = None;
    let mut expected_rgb = None;

    if let Some(paren_start) = raw.find('(') {
        let inside = raw[paren_start + 1..].trim_end_matches(')');
        for part in inside.split(',') {
            let part = part.trim();
            if let Some(td_str) = part.strip_prefix("TD:") {
                if let Ok(v) = td_str.trim().parse::<f32>() {
                    expected_td = Some(v);
                }
            } else if let Some(hex_str) = part.strip_prefix("Hex:") {
                let hex_clean = hex_str.trim().trim_start_matches('#').to_uppercase();
                if let Some(rgb) = parse_hex_rgb(&hex_clean) {
                    expected_rgb = Some(rgb);
                    expected_hex = Some(hex_clean);
                }
            }
        }
    }

    // For Black (TD: unknown/very low), provide 0,0,0 expected color if not explicitly written.
    if name.eq_ignore_ascii_case("black") && expected_rgb.is_none() {
        expected_rgb = Some((0, 0, 0));
        expected_hex = Some("000000".to_string());
    }

    FilamentGroundTruth {
        name,
        raw_header: raw,
        expected_td,
        expected_rgb,
        expected_hex,
    }
}

fn parse_channel_baselines_info(line: &str, baselines: &mut ChannelBaselines) {
    // Example:
    // [INFO ] Channel baselines: R=53507.0 (level 224), G=54182.0 (level 255), B=53269.0 (level 255), detect=54694.0 (level 96)
    let Some(rest) = line.split("Channel baselines:").nth(1) else {
        return;
    };
    for segment in rest.split(',') {
        let seg = segment.trim();
        let Some((k, val_part)) = seg.split_once('=') else {
            continue;
        };
        let mut parts = val_part.split_whitespace();
        let Some(val_str) = parts.next() else {
            continue;
        };
        let Ok(val) = val_str.parse::<f32>() else {
            continue;
        };
        let level = seg
            .split("(level ")
            .nth(1)
            .and_then(|s| s.trim_end_matches(')').parse::<u8>().ok());

        match k.trim() {
            "R" => {
                baselines.red = val;
                if let Some(l) = level {
                    baselines.red_level = l;
                }
            }
            "G" => {
                baselines.green = val;
                if let Some(l) = level {
                    baselines.green_level = l;
                }
            }
            "B" => {
                baselines.blue = val;
                if let Some(l) = level {
                    baselines.blue_level = l;
                }
            }
            "detect" => {
                baselines.detect_baseline = val;
                if let Some(l) = level {
                    baselines.detect_level = l;
                }
            }
            _ => {}
        }
    }
}

/// Parse the contents of `logged_data.txt` into structured sessions and sweeps.
pub fn parse_logged_data(input: &str) -> LoggedDataset {
    let mut baselines = ChannelBaselines::default_hardware();
    let mut sessions: Vec<FilamentSession> = Vec::new();
    let mut detection_cycles: Vec<DetectionCycleRecord> = Vec::new();

    let mut current_gt: Option<FilamentGroundTruth> = None;
    let mut current_sweeps: BTreeMap<u32, Vec<RawSampleRow>> = BTreeMap::new();
    let mut pending_detect_rows: Vec<RawSampleRow> = Vec::new();

    let flush_session = |current_gt: &mut Option<FilamentGroundTruth>,
                         current_sweeps: &mut BTreeMap<u32, Vec<RawSampleRow>>,
                         baselines: ChannelBaselines,
                         sessions: &mut Vec<FilamentSession>| {
        if let Some(gt) = current_gt.take() {
            let sweeps: Vec<FilamentSweep> = std::mem::take(current_sweeps)
                .into_iter()
                .filter(|(_, rows)| rows.iter().any(|r| r.led != "off" && r.led != "detect"))
                .map(|(sweep_id, rows)| FilamentSweep { sweep_id, rows })
                .collect();
            if !sweeps.is_empty() {
                sessions.push(FilamentSession {
                    ground_truth: gt,
                    baselines,
                    sweeps,
                });
            }
        } else {
            current_sweeps.clear();
        }
    };

    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("[INFO ] Baseline dark:") {
            flush_session(
                &mut current_gt,
                &mut current_sweeps,
                baselines,
                &mut sessions,
            );
            if let Some(v) = line
                .split("Baseline dark:")
                .nth(1)
                .and_then(|s| s.trim().parse::<f32>().ok())
            {
                baselines.dark = v;
            }
            continue;
        }

        if line.starts_with("[INFO ] Channel baselines:") {
            parse_channel_baselines_info(line, &mut baselines);
            continue;
        }

        if line.starts_with("# boot baselines:") {
            for token in line.trim_start_matches("# boot baselines:").split_whitespace() {
                if let Some(v) = token.strip_prefix("detect_level=") {
                    if let Ok(lvl) = v.parse::<u8>() {
                        baselines.detect_level = lvl;
                    }
                } else if let Some(v) = token.strip_prefix("detect_baseline=") {
                    if let Ok(b) = v.parse::<f32>() {
                        baselines.detect_baseline = b;
                    }
                }
            }
            continue;
        }

        if line.starts_with("# INSERTED") {
            flush_session(
                &mut current_gt,
                &mut current_sweeps,
                baselines,
                &mut sessions,
            );
            current_gt = Some(parse_inserted_header(line));
            continue;
        }

        if line.starts_with("# MEASURED")
            || line.starts_with("# REMOVED")
            || line.starts_with("[INFO ] VEML7700 initialized")
            || line.starts_with("# td-free data log")
        {
            flush_session(
                &mut current_gt,
                &mut current_sweeps,
                baselines,
                &mut sessions,
            );
            continue;
        }

        if line.starts_with("D,") {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 10 {
                continue;
            }
            let Ok(sweep) = parts[1].parse::<u32>() else {
                continue;
            };
            let Ok(millis) = parts[2].parse::<u64>() else {
                continue;
            };
            let led = parts[3].to_string();
            let Ok(r) = parts[4].parse::<u8>() else {
                continue;
            };
            let Ok(g) = parts[5].parse::<u8>() else {
                continue;
            };
            let Ok(b) = parts[6].parse::<u8>() else {
                continue;
            };
            let Ok(sample) = parts[7].parse::<usize>() else {
                continue;
            };
            let Ok(als_raw) = parts[8].parse::<u32>() else {
                continue;
            };
            let Ok(white_raw) = parts[9].parse::<u32>() else {
                continue;
            };

            let row = RawSampleRow {
                sweep,
                millis,
                led: led.clone(),
                r,
                g,
                b,
                sample,
                als_raw,
                white_raw,
            };

            if current_gt.is_some() {
                current_sweeps.entry(sweep).or_default().push(row);
            } else {
                pending_detect_rows.push(row);
            }
        }
    }

    flush_session(
        &mut current_gt,
        &mut current_sweeps,
        baselines,
        &mut sessions,
    );

    // Pair up `off` + `detect` chunks from `pending_detect_rows` into DetectionCycleRecords.
    let mut i = 0;
    while i < pending_detect_rows.len() {
        if pending_detect_rows[i].led == "off" {
            let sweep = pending_detect_rows[i].sweep;
            let mut off_als = Vec::new();
            let mut off_white = Vec::new();
            while i < pending_detect_rows.len() && pending_detect_rows[i].led == "off" {
                off_als.push(pending_detect_rows[i].als_raw as f32);
                off_white.push(pending_detect_rows[i].white_raw as f32);
                i += 1;
            }
            let mut det_als = Vec::new();
            let mut det_white = Vec::new();
            let mut det_lvl = baselines.detect_level;
            while i < pending_detect_rows.len() && pending_detect_rows[i].led == "detect" {
                det_lvl = pending_detect_rows[i].r;
                det_als.push(pending_detect_rows[i].als_raw as f32);
                det_white.push(pending_detect_rows[i].white_raw as f32);
                i += 1;
            }
            if !det_white.is_empty() {
                detection_cycles.push(DetectionCycleRecord {
                    sweep,
                    off_als: median_f32(&mut off_als),
                    off_white: median_f32(&mut off_white),
                    detect_level: det_lvl,
                    detect_als: median_f32(&mut det_als),
                    detect_white: median_f32(&mut det_white),
                });
            }
        } else {
            i += 1;
        }
    }

    LoggedDataset {
        baselines,
        detection_cycles,
        sessions,
    }
}
