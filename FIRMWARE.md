# Firmware Overview

This document describes how the td-free firmware works: the measurement
principle, the signal processing pipeline, calibration, and the software
architecture. See `API.md` for the HTTP API and `README.md` for general
project info.

## Hardware

| Component | Interface | Purpose |
|---|---|---|
| ESP32-C3 | - | Main controller, Wi-Fi AP/STA, web server |
| VEML7700 | I²C (GPIO6 SDA, GPIO5 SCL) | Broadband light sensor |
| WS2812B (GPIO7) | RMT channel 1 | Measurement illumination, sits over the sensor |
| WS2812B (GPIO4) | RMT channel 0 | Status indication |

The filament is inserted **between** the measurement LED and the sensor.
All quantities (transmission distance *td* and color) are derived from how
much LED light passes through the filament.

## Measurement principle

The device has only **one broadband sensor** (no color channels), so color
is produced *by the light source*: the WS2812B illuminates sequentially in
red, green and blue and the sensor measures how much of each channel gets
through the filament.

1. **Baselines** (taken at boot and periodically while idle): for each
   channel, the LED level is auto-exposed (brightest level whose air
   reading lands in 15k–55k counts, keeping the sensor linear) and the
   air reading is stored as the channel baseline. An LED-off dark reading
   captures ambient.
2. **Measurement**: for each channel, the LED is set to its stored level,
   the sensor reads both the **ALS register** (550 nm narrowband photopic
   photodiode) and the **WHITE register** (broadband photodiode), and the
   per-cycle LED-off ambient is subtracted from both.
3. **Dual-band spectral separation (`td-calc`)**:
   - Under 470 nm Blue illumination, unshifted geometric/TiO2 scatter has
     a fixed `ALS / WHITE ≈ 0.964` ratio, whereas wavelength-shifted dye
     fluorescence (e.g. Magenta) has `ALS / WHITE ≈ 0.396`. This separates
     unfiltered geometric scatter `b_scat` from bulk blue transmission/fluorescence `b_fluor`.
   - Proportional geometric scatter (`0.135 · b_scat` on Red, `0.115 · b_scat`
     on Green) and dark leakage are subtracted from the Red and Green channels.
4. **td** (HueForge scale, 0.1 = opaque to 100 = fully transparent):

   ```
   t_eff = max(wr · r_corr, 1.2 · wg · g_corr) + 3.5 · wb · b_fluor
   td = (0.20 × t_eff ^ 0.46 × 100) × m + b
   ```

5. **Color**: subtracts the white-TiO2 scatter floor per channel, uses the
   525 nm Green and 470 nm Blue `ALS / WHITE` wavelength-shift signals to
   recover yellow/magenta pigment absorption, normalizes by the dominant
   channel (with `0.85` exponent to invert multiple-scattering compression),
   and applies `RGBMultipliers`.

Because each channel is measured against its own baseline, LED brightness,
LED color and sensor gain cancel out - only stability between baseline and
measurement matters.

### Noise handling

- Sensor config: gain ×2, integration time 100 ms.
- Samples are spaced 120 ms apart so each sample comes from an
  **independent integration window** (the sensor latches its registers at
  the end of each window); the median of 3 then rejects transient drops.
- Readings wait 150 ms after an LED change so the sampled window lies
  (mostly) after the change.
- Wi-Fi TX power is capped at 10 dBm: radio current spikes otherwise sag
  the supply rail and dim the LED mid-measurement (visible as ghost
  detections).

### Filament detection

Detection mirrors the measurement: dim (auto-exposed) white light, an
LED-off ambient read, and comparison of the *net* reading against a
baseline taken without a filament:

- reading ≤ `threshold × baseline` (default 0.9) → filament inserted
- 2 consecutive detections are required (debounce)
- unobstructed readings (≥ 97% of baseline) slowly re-track the baseline
  (EMA, α = 0.2) so LED aging, temperature and relocation self-heal
- after ~50 idle cycles the full baselines are re-taken

A sanity check rejects measurements that transmit > 90% on average
(baseline drift or a detection misfire) instead of publishing garbage.

### Known physics limitation

For opaque, strongly scattering filaments, a large part of the signal is
light scattered inside the PLA bulk rather than spectrally transmitted
light (diagnosed via ALS/WHITE spectral-ratio measurements). This part
depends on insertion geometry and cannot be removed by calibration. The
**black reference** (below) removes the reproducible portion of it; for
the rest, a laterally offset LED/sensor geometry is the mechanical
mitigation.

## Calibration

Persisted in NVS, editable via the web UI:

- **Color calibration** (`POST /config/auto-calibrate`): insert a
  reference filament with a known color, pick the target RGB; a hill
  climbing optimizer (global brightness first, then per-channel
  multipliers) adjusts the multipliers so the reported color matches.
- **Black reference** (`POST /config/black-calibrate`): insert a fully
  opaque filament; the per-channel counts (light leaking around/through
  the filament) are stored and subtracted from all future measurements.
  Re-capture when the housing/geometry changes. Do it before the color
  calibration.

NVS keys: `SET` (settings), `RGB` (multipliers), `BLKRF` (black
reference), `WF_CR` (Wi-Fi credentials). All reads fall back to defaults
if the stored blob no longer matches the struct layout (firmware upgrade).

## Software architecture

Embassy async, `no_std`. Rust workspace: `firmware/` (library + bins).

### Tasks

| Task | File | Role |
|---|---|---|
| `sensor_task` | `tasks/sensors.rs` | The state machine: detect → measure → publish |
| `status_led_task` | `tasks/leds.rs` | Status WS2812B (GPIO4): warmup/wifi/measurement state colors |
| `measure_led_task` | `tasks/leds.rs` | Measurement WS2812B (GPIO7), written linearly (no gamma) |
| `data_update_save_task` | `tasks/states.rs` | Persists settings/multipliers/black-ref to NVS |
| `handle_serial_task` | `tasks/serial.rs` | USB serial JTAG interface |
| `web_task` pool | `tasks/http/` | picoserve HTTP server, SSE stream |

### Signals (embassy-sync)

| Channel | Type | Direction |
|---|---|---|
| `MEASUREMENT_STATE` | Watch | sensor → status LED/frontend (`Warmup`/`Idle`/`FilamentInserted`) |
| `MEASUREMENT_DATA_WATCH` | Watch | sensor → SSE (`td`, `hex_color`, buffer count) |
| `SETTINGS_DATA_WATCH` | Watch | latest settings (threshold, m/b, channel weights, LED brightness) |
| `RGB_MULTIPLIERS_WATCH` | Watch | latest color calibration |
| `BLACK_REF_WATCH` | Watch | latest black reference |
| `CALIBRATE_REF_CHANNEL` | PubSub | HTTP → sensor: color calibration request |
| `BLACK_CALIBRATE_CHANNEL` | PubSub | HTTP → sensor: black reference capture request |
| `MEASURE_LED_CHANNEL` | PubSub | sensor/optics → LED task (`Off`/`Rgb`) |
| `DATA_UPDATE_CHANNEL` | PubSub | HTTP → persistence task |
| `CLIENT_CONNECTED` | Watch | whether an SSE client is listening (LED off and measurements paused without one) |

Sensor loop (`tasks/sensors.rs`), simplified:

```
setup VEML7700 (gain x2, IT 100 ms)          -> state Warmup
take channel baselines (auto-exposure)       -> state Idle
loop:
    wait for an SSE client
    detection cycle (off + dim white)         -> publish Idle or FilamentInserted
    if confirmed inserted:
        2 cycles of: off (ambient) + R + G + B readings -> median buffers
        transmittance (black-ref subtracted)  -> td + hex color
        handle pending black-reference capture
        handle pending color calibration
        publish MeasurementData
    else:
        clear buffers, re-baseline every ~50 idle cycles
```

### Binaries & Crates

- `td-calc` (`td-calc/`) - hardware-independent `#![no_std]` library + host
  CLI/test suite containing the TD, color, detection, median buffer, and
  calibration algorithms. Shared directly by `firmware` on ESP32-C3 and
  runnable natively on the host against `logged_data.txt`:
  - Run tests: `cargo td-test` (or `cd td-calc && cargo test`)
  - Evaluate logged data: `cargo td-eval` (or `cargo td-eval --all-levels`)
- `firmware` (`firmware/src/bin/main.rs`) - the production firmware (web UI, API,
  measurement, storage).
- `data_logger` (`firmware/src/bin/data_logger.rs`) - development tool: no Wi-Fi,
  no server. Waits for a filament insertion (same detection logic), then
  records two full sweeps over an illumination ladder (R/G/B × 255…32,
  white × 255…32, interleaved `off` states) as raw CSV on the USB console
  (`D,...` data lines, `#` marker lines). Used to iterate on the
  measurement algorithm offline without reflashing.

### Sensor helpers

| Module | Content |
|---|---|
| `helpers/v77.rs` | VEML7700 driver config (gain/IT), `sample_median` with independent windows and failure recovery, saturation detection |
| `helpers/optics.rs` | Auto-exposure, channel baselines, filament detection, transmittance/td/color math |
| `helpers/calibration.rs` | Color calibration optimizer (brightness + channel hill climbing) |
| `helpers/median_buffer.rs` | Running median buffers for the measurement channels |
| `helpers/storage/` | NVS (tickv) persistence: read/save/delete for settings, multipliers, black reference, Wi-Fi |
| `tasks/http/` | picoserve app, config endpoints, SSE, network bring-up (AP with captive portal fallback) |
