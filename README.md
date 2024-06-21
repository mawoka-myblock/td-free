# Td-Free

A free and nearly[^1] open source device to measure the td value of your filaments.

The CAD is available in the [onshape workspace](https://cad.onshape.com/documents/e7ec65aec40b24c9a33c1902/w/dc90f86d4d08d2181a707cee/e/a86c6c8c0a4124509901ffce)
(licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)) and the print files can be downloaded from [Printables](https://www.printables.com/model/919380-td-free)

## Comparison to TD-1 by Ajax

| TD-Free                   | TD-1                                     |
| ------------------------- | ---------------------------------------- |
| **Small**                 | Slightly bigger                          |
| **Cheap**                 | More expensive                           |
| Only td, no color         | **Measures color and td**                |
| Only diy                  | **Pre-builts and kits available**        |
| Needs a power adapter     | **Runs off of USB**                      |
| WiFi-enabled              | Only via usb and screen                  |
| **Open-Source Code**      | Closed source                            |
| **No license required**   | License needs to be purchased            |
| No integrations available | **Perfectly integrated into HueForge**   |
| **CAD available**         | CAD not available                        |
| only for checking samples | **monitor td of spool as it's printing** |

So, in summary, the Td-Free is **cheaper**, **open-source**[^1], **smaller** and **wifi-enabled**.
But if you want a full featured one and have enough money to spend, feel free to buy the parts list and a license.

## Requirements

- Soldering Iron
- Some thin wire to connect everything
- Probably a 24v power supply
- At least black filament

## Shopping List

### 12v-Version

- [COB LED Strip](https://s.click.aliexpress.com/e/_DDqwOPl) (`4000K Natural White`, `12v 528 leds`, any length) (**Choose the 528 LEDs!**)
- [5,5x2,1mm round barrel jack](https://s.click.aliexpress.com/e/_DmneAx5)
- [This VEML7700 board](https://de.aliexpress.com/item/1005004926993351.html)
- [Supermini ESP32-C3](https://s.click.aliexpress.com/e/_DFmIcbN) (make sure to choose the board, not the expansion board!)
- [Mini360 Buck Step-Down](https://de.aliexpress.com/item/1005004872563696.html) (you can also use the one provided in the 24v-version,
  as this one only supports up to 23v.)

### 24v-Version

- [COB LED Strip](https://s.click.aliexpress.com/e/_DDqwOPl) (`4000K Natural White`, `24v 528 leds`, any length) (**Choose the 528 LEDs!**)
- [5,5x2,1mm round barrel jack](https://s.click.aliexpress.com/e/_DmneAx5)
- [This VEML7700 board](https://de.aliexpress.com/item/1005004926993351.html)
- [Supermini ESP32-C3](https://s.click.aliexpress.com/e/_DFmIcbN) (make sure to choose the board, not the expansion board!)
- [Buck Step-Down converter](https://s.click.aliexpress.com/e/_DEgvDJD)

~ 6€ in parts cost (when only calculating per needed)

## Printing Instructions

- The `Layer` parts and the `sensor holder` should be printed with black filament, the rest doesn't matter.
  Select a cool color combination!
- Use the "auto orient" feature of your slicer!

## Assembly instructions

At first, I'd recommend soldering all the components together.

- Solder the power connector to the step-down converter board (back one most likely positive)
- Solder the LED strip to the connector or the step-down converter input as well, so both get 24/12v.
- Solder plus and minus output of the step-down converter to the ESP32 and to the VEML 7700 board.
- Solder SDA to pin 8 on the ESP32 and SCL to pin 10 on the ESP.
- **Flash the ESP (instructions below)**
- Stuff everything into the case and press the power socket into the case (make sure not to short anything).
- Put the lid on and make sure that the corner with the extrusions is above the power socket to hold it in place.

## Flashing instructions

1. Download the latest build [here](https://nightly.link/mawoka-myblock/td-test/workflows/platformio_build/main/firmware-esp32c3.bin.zip).
   They are directly built by GitHub.
2. Extract the downloaded zip file.
3. Install esp-tool.
4. Flash the esp32: `esptool.py -b 230400 write_flash 0x0 firmware.bin`

## Usage

- Plug it into power
- Wait some seconds until you find a wifi hotspot called "Td-Free" and connect to it.
- A website should now open, which should refresh automatically every second
- Insert your filament and read the td value

> [!NOTE]  
> Make sure **no filament is inserted at startup**, as it calibrates on startup.

[^1]: nearly, as the CAD is licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), but the code is open source.
