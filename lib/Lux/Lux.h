#ifndef COLOR_SENSOR_H
#define COLOR_SENSOR_H

#include <Wire.h>
#include <utility>

std::pair<float, float> temperature_and_lux_dn40(float gain, float glass_attenuation, float color_raw[4], uint8_t atime);

#endif // COLOR_SENSOR_H
