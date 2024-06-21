# Td-Free

A free and open source device to measure the td value of your filaments



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



