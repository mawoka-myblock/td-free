# Accuracy improvements

These changes improve how the Td-Free measures TD and color. Measured on one device against four Polymaker
spools, the average TD error went from **47% to 12%** and the average color error from **ΔE 59 to 10**.
This document lists what was changed, why, and what each step did to the results.

All measurements were taken on 23 and 24 October 2026 with one Td-Free (ESP32-C3, VEML7700 light sensor,
VEML3328 color sensor, cold-white COB lamp, WS2812 status LED), starting from commit `5e67e73`.

## Results at a glance

| | Stock firmware | With these changes |
|---|---|---|
| Average TD error to the spool label | 47% | 12% |
| Worst TD error | 56% (yellow, natural) | 24% (cyan) |
| Average color error to the spool label (ΔE CIE76) | 59 | 10 |
| Repeatability, natural inserted 5 times | not measured | ±1.7% (TD 21.4 to 22.2) |

## Reference filaments

All four are Polymaker Panchroma PLA Translucent, 1.75 mm, 1 kg. TD and hex color are printed on the spool
label and serve as the reference. They are manufacturer values, not ground truth.

| Color | SKU | Batch | TD on the label | Hex on the label |
|---|---|---|---|---|
| Translucent Magenta | CA02002 | A7818330366 | 7.8 | `#D93B90` |
| Translucent Cyan | CA02001 | not recorded | 8.0 | `#08ABFB` |
| Translucent Yellow | CA02003 | A7230750753 | 14 | `#F9ED3D` |
| Translucent Natural | CA02037 | A7512690174 | 21 | `#E8E6D0` |

- Product page: [Polymaker Panchroma Translucent PLA](https://shop.polymaker.com/en-eu/collections/pla-filament/products/translucent-pla)
- Cyan, magenta and yellow come from the [Polymaker CMYK Lithophane Bundle](https://www.3djake.com/polymaker/cmyk-lithophane-bundle).
  Its fourth spool, PolyLite PLA White, was not used. The natural spool was bought separately.

## What was changed

### 1. The light sensor was overloaded

The stock firmware runs the VEML7700 at gain ×1 and 100 ms integration time. With the lamp at 100% and an
empty chamber the sensor sat at its 16 bit limit (65535 counts, reported as 4404 lx), so the reference that
every TD is divided by was clipped.

A sweep over gain, integration time and lamp level showed:

- The lamp output is proportional to its PWM duty within 0.1% (checked with the VEML3328, which stays
  linear over the whole range).
- At gain ×1 the light sensor is compressed by a factor of 4 to 5 even at the lowest lamp step.
- Gain ×1/4 and ×1/8 track each other at exactly 2.000 : 1 and follow the VEML3328 within a few percent.
- The driver's built-in correction for high light levels (Vishay's polynomial for gain ×1/4 and ×1/8)
  inflated the full-lamp reading by up to 2.8 times although the sensor is nearly linear.

The sensor now runs at gain ×1/8 and 25 ms. The empty chamber uses about 22% of the range, the noise is
0.08%, and counts are converted to lux linearly without the polynomial.
(`firmware/src/helpers/v77.rs`: `configure_als`, `read_als_lux`)

### 2. TD from the broadband channel with a fitted formula

The stock firmware computes TD as 10 × (light through the filament ÷ light through the empty chamber) from
the ALS channel, plus a boost for red-dominant filaments. The ALS channel follows the sensitivity of the
human eye and barely sees the blue and red light magenta lets through, so it read magenta 4 times darker
than cyan although both have a TD of about 8.

TD is now computed from the broadband WHITE channel of the VEML7700, which treats magenta and cyan far more
evenly:

```
TD = 32.67 × T^0.669        T = WHITE channel with filament ÷ WHITE channel with empty chamber
```

The formula was fitted to the four reference spools (rms error 15%). It gives about 0 for an opaque filament,
but below a TD of about 7 it is an extrapolation. The power function uses `libm`, because the `micromath`
approximation was up to 7% off at these values. The red-dominant boost was removed.
(`firmware/src/tasks/sensors.rs`: `td_from_white_transmission`)

### 3. Cleaner measurements

- **Status LED off while measuring.** The WS2812 status LED leaks into the color sensor. It was yellow
  while the empty-chamber reference was taken and cyan while a filament was measured. It now stays dark
  during warm-up and while a filament is inserted, and shows the Wi-Fi state otherwise. Switching it off
  changed for example the yellow spool's reading from `#807E23` to `#FFFF5C`.
- **First reading 1 s after the filament is detected**, so readings of a half inserted filament don't end
  up in the median.
- **Color sensor reference at 50% lamp, doubled**, because its clear channel clips at 100%. The lamp is
  linear, so this is exact.

### 4. Color calibration with known filaments

The stock color code scaled the brightness by the light passing the filament, relative to a stored
reference whose default (50) made every color `#FFFFFF`. After a calibration it turned translucent colors
almost black instead. Its search for the channel factors stopped as soon as a channel was clipped, and the
result was never saved.

Colors are now computed in linear light: the white balanced sensor color is multiplied by a 3×3 matrix and
encoded to sRGB. The matrix is fitted to up to 8 reference filaments stored on the device:

- no reference: white balance only (already better than the stock default)
- 1 or 2 references: one factor per channel (white point)
- 3 or more references: a full 3×3 matrix, pulled towards the per-channel solution so that a handful of
  references doesn't overfit

On the Calibrate page you insert a filament, enter its hex color and add it as a reference. Each reference
shows the spool color next to the color the device now reports. Translucent filaments keep their hue at
full brightness, because they look bright although they let through only a few percent of the light.
(`firmware/src/helpers/calibration.rs`, `firmware/src/tasks/http/config.rs`, Calibrate page)

### 5. Bug fixes

- **Settings could not be saved** while the Spoolman IP and port were empty: the empty port failed the
  format check and disabled the Save button.
- **The calibration result was lost**: it only lived in RAM. The HTTP reply returned the old values right
  away instead of the result, and a calibration requested without a filament fired later on whichever
  filament was inserted next.
- **The hex color on the dashboard was invisible** in light mode (black text on the black chip).
- **A TD of 100 or more, or −10 or less, crashed the firmware** while formatting it (possible with the
  Algo m/b settings). It is now clamped.
- "LED Brightness" in the settings is now called "Status LED Brightness", since it never affected the lamp.

### 6. Diagnostics

The serial log (`cargo run --release`, defmt) now shows at boot the empty-chamber reference of every
channel and the loaded calibration. For every measurement it shows a `RAW` line with all channels (ALS, WHITE,
clear, R, G, B) and their transmission, with a warning when a channel clips.

## Results per step

TD read from the web page after each change, except step 3, which comes from the serial log.

| Step | Change | Magenta (7.8) | Cyan (8.0) | Yellow (14) | Natural (21) | Avg. error |
|---|---|---|---|---|---|---|
| 1 | Stock firmware | 11.7 (+50%) | 6.0 (−25%) | 6.2 (−56%) | 9.2 (−56%) | 47% |
| 2 | Status LED off | 11.6 (+49%) | 5.9 (−26%) | 6.2 (−56%) | 9.9 (−53%) | 46% |
| 3 | Sensor no longer overloaded, old formula | 2.1 (−73%) | 1.9 (−76%) | 2.1 (−85%) | 5.3 (−75%) | 77% |
| 4 | WHITE channel and fitted formula | 7.0 (−10%) | 9.0 (+12%) | 10.5 (−25%) | 21.1 (+0%) | 12% |
| 5 | Exact power function | 7.3 (−6%) | 9.9 (+24%) | 11.5 (−18%) | 21.1 (+0%) | 12% |

Step 3 was a transition. Once the sensor was no longer overloaded, the old formula no longer matched its
scale. The data of that step was used to fit the formula of step 4.

Color reported by the device, with the color difference ΔE (CIE76) to the spool label:

| Step | Magenta | Cyan | Yellow | Natural | Avg. ΔE |
|---|---|---|---|---|---|
| Stock firmware | `#34102F` 60 | `#133BD4` 71 | `#807E23` 53 | `#8CB7FF` 54 | 59 |
| Status LED off | `#922378` 24 | `#3381FF` 36 | `#FFFF5C` 9 | `#FFFFFF` 15 | 21 |
| Old calibration, on natural | `#190614` 76 | `#0F2791` 62 | `#6E5C19` 68 | `#EBE6FF` 24 | 58 |
| White point from natural | `#FF476C` 33 | `#2A6FD0` 30 | `#FFD31F` 16 | `#EAEAD0` 2 | 20 |
| Color matrix, all four as references | `#DB248D` 7 | `#2E95EF` 13 | `#FBE500` 10 | `#DEDFDE` 11 | 10 |

The good result with the status LED off was probably helped by a calibration still active in memory at
the time; that could not be checked.

## Known limits

- **One device, four spools.** The TD formula and the color matrix were fitted to the same four spools they
  are evaluated on. Below a TD of about 7 the formula is an extrapolation.
- **Cyan reads 24% high and yellow 18% low.** They let through almost the same share of light on every
  channel, so a formula based on transmission alone can't separate them. More spools with a printed TD would
  allow a formula that also uses the color channels.
- **The color matrix is unproven for other colors.** Fitted on three of the spools, it predicted the fourth
  with an average ΔE of 22, about as good as the white point alone. Every additional reference with a known
  hex color, which doesn't need a TD, makes it more reliable.
- **The lamp drifts by 1 to 2%** while a filament stays inserted, most likely because it loses a little
  output as it warms up. The empty-chamber reference is only taken at power-up.
- **Very dark filaments read high.** Below about 6% of the full lamp light the ALS reads 12% to 49% more than
  it should.

## Measuring

1. Power the device with an empty chamber and the lid closed. The reference is taken at power-up.
2. Insert the filament and wait 30 to 45 seconds until the value settles.
3. Pull it out completely before inserting the next one.

For color, add at least three filaments with a known hex color as references on the Calibrate page,
ideally including a white or natural one.
