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
But if you want a fully featured one and have enough money to spend, feel free to buy the parts list and a license for a td-1.

## Requirements

- Soldering Iron
- Some thin wire to connect everything
- Double-sided tape
- At least black filament

## Shopping List

- [COB LED Strip](https://s.click.aliexpress.com/e/_DlNXKdH) (`Cold white 10pcs`)
- [5,5x2,1mm round barrel jack](https://s.click.aliexpress.com/e/_DmneAx5)
- [This VEML7700 board](https://de.aliexpress.com/item/1005004926993351.html)
- [Supermini ESP32-C3](https://de.aliexpress.com/item/1005005877531694.html) (make sure to choose the board, not the expansion board!)


~ 6€ in parts cost (when only calculating per needed)

## Printing Instructions

- The `Layer` parts and the `sensor holder` should be printed with black filament
- Select a cool color combination!
- If you have, print the base out of a temperature resistant filament like PETG. PLA works but see notes later on!
- Use the "auto orient" feature of your slicer!

## Assembly instructions

At first, I'd recommend soldering all the components together.

- Solder the led module to the ESP32, just like the VEML 7700. The LED module must be soldered at the side with the resistor, otherwise it won't work!
- Solder SDA to pin 8 on the ESP32 and SCL to pin 10 on the ESP.
- **Flash the ESP (instructions below)**
- Stick the ESP32 with some double-sided tape onto the lid making sure it's centered!
- Use some double-sided tape to stick the LED module onto the cross and try to center it!
- Stuff everything into the case (make sure not to short anything).
- Put the lid on.

## Flashing instructions

1. Download the latest build [here](https://nightly.link/mawoka-myblock/td-free/workflows/platformio_build/main/esp32c3-4mb.zip).
   They are directly built by GitHub.
2. Extract the downloaded zip file.
3. [Install esp-tool](https://docs.espressif.com/projects/esptool/en/latest/esp32/#quick-start).
4. Flash the esp32: `esptool.py -b 230400 write_flash 0x0 td-free.bin` (If you can't flash it, try holding down the `BOOT` buton while plugging it into the PC)

## Usage

- Plug it in.
- Wait some seconds until you find a wifi hotspot called "Td-Free" and connect to it.
- A website should now open, which should refresh automatically every second
- Insert your filament and read the td value

> [!NOTE]  
> Make sure **no filament is inserted at startup**, as it calibrates on startup.


> [!NOTE]  
> If you print the `Base` out of PLA, I'd recommend to not keep the TD-free plugged in for more than 5 minutes as the LED module get quite hot making the PLA soft. There won't be any other problems, just the PLA softening.


## Changes in V2
- Now using another LED part than the strip that runs from 5v
- Step-Up converter not needed anymore
- Power via 5v USB-C
- Improved case and lid

### Upgrade from V1
At first, the only improvement you'll get it that you can power it with 5v. For the upgrade, you'll need the following:
- The LED modules listed in the shopping list
- Print the new `Base` and `Lid` parts

Then, desolder the cables from the output of the step-down converter and solder the cables from the VEML to the ESP32. Then, solder the two free cables to the LED module, making sure to connect plus and minus accordingly to the side with the resistor!

[^1]: nearly, as the CAD is licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), but the code is open source.
