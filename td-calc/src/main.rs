use std::env;
use std::fs;
use std::path::PathBuf;

use td_calc::calibration::calculate_rgb_distance;
use td_calc::log_parser::{SampleAggregation, parse_logged_data};
use td_calc::optics::{
    ChannelBaselines, evaluate_measurement, td_from_transmittance, transmittance, update_detection,
};
use td_calc::{AlgoAdjustment, BlackReference, RGBMultipliers};

fn find_default_log_file() -> PathBuf {
    let candidates = [
        PathBuf::from("testdata/logged_data.txt"),
        PathBuf::from("td-calc/testdata/logged_data.txt"),
        PathBuf::from("logged_data.txt"),
        PathBuf::from("../td-calc/testdata/logged_data.txt"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    PathBuf::from("testdata/logged_data.txt")
}

fn print_help() {
    println!("Usage: td-calc [OPTIONS] [LOG_FILE]");
    println!();
    println!("Evaluate the td-free TD & color calculation algorithm against precollected sensor logs.");
    println!();
    println!("Options:");
    println!("  --level <255|192|128|64|32>  LED drive level to evaluate (default: 255)");
    println!("  --all-levels                 Evaluate across 255, 192, and 128 levels");
    println!("  --agg <settled|first3|all>   Sample aggregation mode (default: settled)");
    println!("  --use-black-ref              Use the Black filament session as BlackReference");
    println!("  --wr <f32>                   Red TD weight multiplier (default: 1.0)");
    println!("  --wg <f32>                   Green TD weight multiplier (default: 1.0)");
    println!("  --wb <f32>                   Blue TD weight multiplier (default: 1.0)");
    println!("  --m <f32>                    Linear TD slope adjustment (default: 1.0)");
    println!("  --b <f32>                    Linear TD offset adjustment (default: 0.0)");
    println!("  -h, --help                   Show this help message");
}

fn legacy_eval(
    r_white: f32,
    g_white: f32,
    b_white: f32,
    level: u8,
    baselines: &ChannelBaselines,
) -> (f32, (u8, u8, u8)) {
    let (br, bg, bb) = baselines.scaled_for_levels(level, level, level);
    let scaled_base = ChannelBaselines {
        red: br,
        green: bg,
        blue: bb,
        ..*baselines
    };
    let t = transmittance(
        (r_white, g_white, b_white),
        &scaled_base,
        &BlackReference::default(),
    );
    let td = td_from_transmittance(t, (1.0, 1.0, 1.0));
    let r = (t.0 * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (t.1 * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (t.2 * 255.0).round().clamp(0.0, 255.0) as u8;
    (td, (r, g, b))
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut file_path: Option<PathBuf> = None;
    let mut level: u8 = 255;
    let mut all_levels = false;
    let mut use_black_ref = false;
    let mut agg = SampleAggregation::SettledMedian;
    let mut algo = AlgoAdjustment::default();
    let multipliers = RGBMultipliers::default();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "--level" => {
                i += 1;
                level = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .expect("Invalid --level value");
            }
            "--all-levels" => {
                all_levels = true;
            }
            "--use-black-ref" => {
                use_black_ref = true;
            }
            "--agg" => {
                i += 1;
                agg = match args.get(i).map(|s| s.as_str()) {
                    Some("settled") => SampleAggregation::SettledMedian,
                    Some("first3") => SampleAggregation::First3Median,
                    Some("all") => SampleAggregation::AllMedian,
                    _ => panic!("Invalid --agg value (expected settled, first3, or all)"),
                };
            }
            "--wr" => {
                i += 1;
                algo.wr = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .expect("Invalid --wr");
            }
            "--wg" => {
                i += 1;
                algo.wg = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .expect("Invalid --wg");
            }
            "--wb" => {
                i += 1;
                algo.wb = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .expect("Invalid --wb");
            }
            "--m" => {
                i += 1;
                algo.m = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .expect("Invalid --m");
            }
            "--b" => {
                i += 1;
                algo.b = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .expect("Invalid --b");
            }
            other if !other.starts_with('-') => {
                file_path = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Unknown argument: {}", other);
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let path = file_path.unwrap_or_else(find_default_log_file);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));

    let dataset = parse_logged_data(&content);

    println!("==========================================================================================");
    println!(
        "td-calc offline evaluation | file: {} | sessions: {} | agg: {:?}",
        path.display(),
        dataset.sessions.len(),
        agg
    );
    println!(
        "Baselines: R={:.0} (lvl {}), G={:.0} (lvl {}), B={:.0} (lvl {}), detect={:.0} (lvl {})",
        dataset.baselines.red,
        dataset.baselines.red_level,
        dataset.baselines.green,
        dataset.baselines.green_level,
        dataset.baselines.blue,
        dataset.baselines.blue_level,
        dataset.baselines.detect_baseline,
        dataset.baselines.detect_level
    );
    println!("==========================================================================================");

    if !dataset.detection_cycles.is_empty() {
        let mut det_base = dataset.baselines;
        let mut streak = 0u32;
        println!("\n--- Filament Detection Replay ---");
        for (idx, cyc) in dataset.detection_cycles.iter().enumerate() {
            let net = cyc.detect_white - cyc.off_white;
            let (blocked, _) = update_detection(net, &mut det_base, algo.threshold);
            streak = if blocked { streak + 1 } else { 0 };
            println!(
                "  Cycle {}: off={:.1}, detect={:.1}, net={:.1} / baseline={:.1} -> blocked={} (streak={}, confirmed={})",
                idx,
                cyc.off_white,
                cyc.detect_white,
                net,
                det_base.detect_baseline,
                blocked,
                streak,
                streak >= 2
            );
        }
    }

    let black_ref = if use_black_ref {
        dataset
            .sessions
            .iter()
            .find(|s| s.ground_truth.name.eq_ignore_ascii_case("black"))
            .and_then(|s| s.combined_spectral_readings(level, agg))
            .map(|sr| BlackReference {
                r: sr.red.white,
                g: sr.green.white,
                b: sr.blue.white,
            })
            .unwrap_or_default()
    } else {
        BlackReference::default()
    };

    let levels_to_run = if all_levels {
        vec![255u8, 192, 128]
    } else {
        vec![level]
    };

    for &lvl in &levels_to_run {
        println!(
            "\n--- Combined Session Results (LED level = {}) ---",
            lvl
        );
        println!(
            "{:<10} | {:>7} {:>7} {:>7} {:>7} | {:<9} {:<9} {:<9} {:>7} | {:>14}",
            "Filament",
            "Exp TD",
            "New TD",
            "TD Err",
            "Old TD",
            "Exp Hex",
            "New Hex",
            "Old Hex",
            "RGB Err",
            "Sweep 0 / 1 TD"
        );
        println!("{}", "-".repeat(106));

        let mut td_sq_err = 0.0f32;
        let mut td_old_sq_err = 0.0f32;
        let mut td_count = 0usize;
        let mut rgb_err_sum = 0.0f32;
        let mut rgb_count = 0usize;

        for session in &dataset.sessions {
            let Some(readings) = session.combined_spectral_readings(lvl, agg) else {
                continue;
            };
            let res =
                evaluate_measurement(&readings, &session.baselines, &black_ref, &algo, &multipliers);
            let (old_td, old_rgb) = legacy_eval(
                readings.red.white,
                readings.green.white,
                readings.blue.white,
                lvl,
                &session.baselines,
            );

            let sw_tds: Vec<String> = session
                .sweeps
                .iter()
                .filter_map(|sw| sw.spectral_readings_at_level(lvl, agg))
                .map(|sr| {
                    let r = evaluate_measurement(
                        &sr,
                        &session.baselines,
                        &black_ref,
                        &algo,
                        &multipliers,
                    );
                    format!("{:.2}", r.td)
                })
                .collect();

            let exp_td_str = session
                .ground_truth
                .expected_td
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "~0.10".to_string());

            let td_err_str = if let Some(exp_td) = session.ground_truth.expected_td {
                let err = res.td - exp_td;
                td_sq_err += err * err;
                let old_err = old_td - exp_td;
                td_old_sq_err += old_err * old_err;
                td_count += 1;
                format!("{:+.2}", err)
            } else {
                "   -".to_string()
            };

            let exp_hex_str = session
                .ground_truth
                .expected_hex
                .as_deref()
                .map(|h| format!("#{}", h))
                .unwrap_or_else(|| "   -".to_string());

            let rgb_err_str = if let Some(exp_rgb) = session.ground_truth.expected_rgb {
                let dist = calculate_rgb_distance(res.rgb, exp_rgb);
                rgb_err_sum += dist;
                rgb_count += 1;
                format!("{:.1}", dist)
            } else {
                "   -".to_string()
            };

            println!(
                "{:<10} | {:>7} {:>7.2} {:>7} {:>7.2} | {:<9} #{:<8} #{:02X}{:02X}{:02X}   {:>7} | {:>14}",
                session.ground_truth.name,
                exp_td_str,
                res.td,
                td_err_str,
                old_td,
                exp_hex_str,
                res.hex_color,
                old_rgb.0,
                old_rgb.1,
                old_rgb.2,
                rgb_err_str,
                sw_tds.join(" / ")
            );
        }

        if td_count > 0 {
            let rms_new = (td_sq_err / td_count as f32).sqrt();
            let rms_old = (td_old_sq_err / td_count as f32).sqrt();
            let mean_rgb = if rgb_count > 0 {
                rgb_err_sum / rgb_count as f32
            } else {
                0.0
            };
            println!("{}", "-".repeat(106));
            println!(
                "Summary (lvl {}): TD RMS Error = {:.3} (was {:.3} in old algo) | Mean RGB Euclidean Distance = {:.1}",
                lvl, rms_new, rms_old, mean_rgb
            );
        }
    }
}
