#![no_std]

#[cfg(any(test, feature = "std"))]
extern crate std;

#[cfg(feature = "defmt")]
use defmt::Format;
use serde::{Deserialize, Serialize};

pub mod calibration;
pub mod median_buffer;
pub mod optics;

#[cfg(any(test, feature = "std"))]
pub mod log_parser;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct RGBMultipliers {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub brightness: f32,
    pub td_reference: f32, // TD value at calibration time
    pub reference_r: u8,   // Reference red value (0-255)
    pub reference_g: u8,   // Reference green value (0-255)
    pub reference_b: u8,   // Reference blue value (0-255)
}

impl Default for RGBMultipliers {
    fn default() -> Self {
        Self {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            brightness: 1.0,
            td_reference: 50.0,
            reference_r: 127,
            reference_g: 127,
            reference_b: 127,
        }
    }
}

/// Per-channel dark reference (ambient-corrected counts) measured with an
/// opaque filament inserted. Compensates light leaking around the
/// filament through the slot gap, which would otherwise be interpreted
/// as transmission. All zeros = no black reference (disabled).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct BlackReference {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Default for BlackReference {
    fn default() -> Self {
        Self {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        }
    }
}

impl BlackReference {
    pub fn is_enabled(&self) -> bool {
        self.r > 0.0 || self.g > 0.0 || self.b > 0.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct AlgoAdjustment {
    pub b: f32,
    pub m: f32,
    pub threshold: f32,
    /// Per-channel weights for the td calculation.
    pub wr: f32,
    pub wg: f32,
    pub wb: f32,
}

impl Default for AlgoAdjustment {
    fn default() -> Self {
        Self {
            b: 0.0,
            m: 1.0,
            threshold: 0.9,
            wr: 1.0,
            wg: 1.0,
            wb: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;
    use crate::calibration::{CalibrationCommand, auto_calibrate_spectral, calculate_rgb_distance};
    use crate::log_parser::{SampleAggregation, parse_logged_data};
    use crate::optics::{
         evaluate_measurement, td_from_transmittance, transmittance, update_detection,
    };

    const LOGGED_DATA: &str = include_str!("../../logged_data.txt");

    #[test]
    fn parses_all_six_filaments_and_detection_cycles() {
        let ds = parse_logged_data(LOGGED_DATA);
        assert_eq!(ds.sessions.len(), 6);
        assert_eq!(ds.detection_cycles.len(), 2);

        let names: Vec<&str> = ds
            .sessions
            .iter()
            .map(|s| s.ground_truth.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["RED", "Teal", "Lime", "Yellow", "Magenta", "Black"]
        );

        for session in &ds.sessions {
            assert_eq!(
                session.sweeps.len(),
                2,
                "Expected 2 sweeps for {}",
                session.ground_truth.name
            );
        }
    }

    #[test]
    fn detection_triggers_after_two_consecutive_blocked_cycles() {
        let ds = parse_logged_data(LOGGED_DATA);
        let mut baselines = ds.baselines;
        let mut streak = 0u32;

        for cycle in &ds.detection_cycles {
            let net = cycle.detect_white - cycle.off_white;
            let (inserted, _) = update_detection(net, &mut baselines, 0.9);
            if inserted {
                streak += 1;
            } else {
                streak = 0;
            }
        }
        assert_eq!(streak, 2, "Expected 2 consecutive insertion detections");
    }

    #[test]
    fn td_values_match_ground_truth_and_order_correctly() {
        let ds = parse_logged_data(LOGGED_DATA);
        let algo = AlgoAdjustment::default();
        let mult = RGBMultipliers::default();
        let black = BlackReference::default();

        let mut sq_err_sum = 0.0f32;
        let mut count = 0usize;

        for session in &ds.sessions {
            let readings = session
                .combined_spectral_readings(255, SampleAggregation::SettledMedian)
                .expect("readings at 255");
            let res = evaluate_measurement(&readings, &session.baselines, &black, &algo, &mult);

            if let Some(expected_td) = session.ground_truth.expected_td {
                let err = (res.td - expected_td).abs();
                assert!(
                    err <= 0.25,
                    "Filament {} TD error too large: got {:.2}, expected {:.2} (err {:.2})",
                    session.ground_truth.name,
                    res.td,
                    expected_td,
                    err
                );
                sq_err_sum += err * err;
                count += 1;
            } else if session.ground_truth.name.eq_ignore_ascii_case("black") {
                assert!(
                    res.td <= 0.2,
                    "Black TD should be very low, got {:.2}",
                    res.td
                );
            }
        }

        let rms = (sq_err_sum / count as f32).sqrt();
        assert!(rms < 0.15, "Overall TD RMS error too high: {:.3}", rms);
    }

    #[test]
    fn color_readings_match_ground_truth() {
        let ds = parse_logged_data(LOGGED_DATA);
        let algo = AlgoAdjustment::default();
        let mult = RGBMultipliers::default();
        let black = BlackReference::default();

        for session in &ds.sessions {
            let readings = session
                .combined_spectral_readings(255, SampleAggregation::SettledMedian)
                .expect("readings at 255");
            let res = evaluate_measurement(&readings, &session.baselines, &black, &algo, &mult);

            if let Some(expected_rgb) = session.ground_truth.expected_rgb {
                let dist = calculate_rgb_distance(res.rgb, expected_rgb);
                assert!(
                    dist < 50.0,
                    "Filament {} RGB {:?} (#{}) too far from expected {:?} (dist={:.1})",
                    session.ground_truth.name,
                    res.rgb,
                    res.hex_color,
                    expected_rgb,
                    dist
                );
            }
        }
    }

    #[test]
    fn sweep_to_sweep_and_level_consistency() {
        let ds = parse_logged_data(LOGGED_DATA);
        let algo = AlgoAdjustment::default();
        let mult = RGBMultipliers::default();
        let black = BlackReference::default();

        for session in &ds.sessions {
            let sw0 = session.sweeps[0]
                .spectral_readings_at_level(255, SampleAggregation::SettledMedian)
                .unwrap();
            let sw1 = session.sweeps[1]
                .spectral_readings_at_level(255, SampleAggregation::SettledMedian)
                .unwrap();

            let r0 = evaluate_measurement(&sw0, &session.baselines, &black, &algo, &mult);
            let r1 = evaluate_measurement(&sw1, &session.baselines, &black, &algo, &mult);

            let td_diff = (r0.td - r1.td).abs();
            assert!(
                td_diff < 0.10,
                "Sweep-to-sweep TD drift on {}: {:.2} vs {:.2}",
                session.ground_truth.name,
                r0.td,
                r1.td
            );

            let lvl192 = session
                .combined_spectral_readings(192, SampleAggregation::SettledMedian)
                .unwrap();
            let r192 = evaluate_measurement(&lvl192, &session.baselines, &black, &algo, &mult);
            assert!(
                (r0.td - r192.td).abs() < 0.15,
                "Level 255 vs 192 TD drift on {}: {:.2} vs {:.2}",
                session.ground_truth.name,
                r0.td,
                r192.td
            );
        }
    }

    #[test]
    fn spectral_calibration_converges_to_exact_reference_color() {
        let ds = parse_logged_data(LOGGED_DATA);
        let red_session = &ds.sessions[0];
        let readings = red_session
            .combined_spectral_readings(255, SampleAggregation::SettledMedian)
            .unwrap();
        let target = red_session.ground_truth.expected_rgb.unwrap();

        let cal = auto_calibrate_spectral(
            CalibrationCommand {
                target_r: target.0,
                target_g: target.1,
                target_b: target.2,
            },
            &readings,
            &red_session.baselines,
            1.0,
            RGBMultipliers::default(),
        )
        .unwrap();

        let res = evaluate_measurement(
            &readings,
            &red_session.baselines,
            &BlackReference::default(),
            &AlgoAdjustment::default(),
            &cal,
        );
        let dist = calculate_rgb_distance(res.rgb, target);
        assert!(dist <= 2.0, "Calibrated color should match target: got {:?} vs {:?}", res.rgb, target);

        // Ensure legacy functions remain callable and well-behaved
        let t = transmittance((188.0, 112.0, 766.0), &red_session.baselines, &BlackReference::default());
        let legacy_td = td_from_transmittance(t, (1.0, 1.0, 1.0));
        assert!(legacy_td > 0.0);
    }
}
