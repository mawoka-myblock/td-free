#include "Lux.h"


// Define the function with appropriate parameters
std::pair<float, float> temperature_and_lux_dn40(float gain, float glass_attenuation, float color_raw[4], uint8_t atime) {
    // Initial input values
    uint8_t ATIME = atime;
    float ATIME_ms = (256 - ATIME) * 2.4;
    float AGAINx = gain;
    float R = color_raw[0];
    float G = color_raw[1];
    float B = color_raw[2];
    float C = color_raw[3];

    // Device specific values
    float GA = glass_attenuation;
    float DF = 310.0;
    float R_Coef = 0.136;
    float G_Coef = 1.0;
    float B_Coef = -0.444;
    float CT_Coef = 3810;
    float CT_Offset = 1391;

    // Analog/Digital saturation
    float SATURATION = (256 - ATIME > 63) ? 65535 : 1024 * (256 - ATIME);

    // Ripple saturation
    if (ATIME_ms < 150) {
        SATURATION -= SATURATION / 4;
    }

    // Check for saturation and mark the sample as invalid if true
    if (C >= SATURATION) {
        return std::make_pair(NAN, NAN); // Return a pair of NaN
    }

    // IR Rejection
    float IR = (R + G + B - C) / 2.0 > 0 ? (R + G + B - C) / 2.0 : 0.0;
    float R2 = R - IR;
    float G2 = G - IR;
    float B2 = B - IR;

    // Lux Calculation
    float G1 = R_Coef * R2 + G_Coef * G2 + B_Coef * B2;
    float CPL = (ATIME_ms * AGAINx) / (GA * DF);
    CPL = (CPL == 0) ? 0.001 : CPL;
    float lux = G1 / CPL;

    // CT Calculations
    R2 = (R2 == 0) ? 0.001 : R2;
    float CT = CT_Coef * B2 / R2 + CT_Offset;

    return std::make_pair(lux, CT);
}
