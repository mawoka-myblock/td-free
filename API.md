# API Docs


## REST

### GET / App HTML
Gets the main index.html

### GET /app.js App JS
Gets the main app.js

### GET /app.css App CSS
Gets the main app.css

### GET /events/data SSE for data
Gets the data measured by the device.

Event is `measurement_changed` and either contains:

```
no_filament
```

or

```json
{
    "td": "2.5",
    "hex_color": Option<"FFFFFF">,
    "buf_color": Option<12>
}
```

### GET/POST /config/settings Set/Get Config
Sets/Gets settings:

```json5
{
    "led_brightness": 100, // in %
    "algo": {
        "b": 0.0,
        "m": 1.0,
        "threshold": 0.9, // between 0.999 and 0.001
        "wr": 1.0, // red channel weight for td
        "wg": 1.0, // green channel weight for td
        "wb": 1.0 // blue channel weight for td
    }
}
```

### GET/POST /config/rgb Set/Get RGB Multipliers
Sets/Gets rgb multipliers:
```json
{
    "red": 1.0,
    "green": 1.0,
    "blue": 1.0,
    "brightness": 1.0,
    "td_reference": 50.0,
    "reference_r": 127,
    "reference_g": 127,
    "reference_b": 127
}
```

### POST /config/wifi Set Wifi Credentials
```json
{
    "ssid": "dsada",
    "password": "dsads"
}
```

### GET /config/info Get Device Info
```json
{
    "has_color": true,
    "version": "06556+4-3.3"
}
```

### POST /config/auto-calibrate Set Auto-calibrate data
Need client listening to server sent events (`/events/data`)

may throw 428 if no client connected

may throw 408 if internal function timeouted

```json
{
    "target_r": 255,
    "target_g": 255,
    "target_b": 255
}
```

Return RGBMultipliers like `/config/rgb`

### POST /config/black-calibrate Capture black reference
Captures the per-channel dark reference with an **opaque black filament inserted**.
It compensates light leaking around the filament, which would otherwise be
reported as transmission. The reference is persisted and subtracted from all
future measurements.

Need client listening to server sent events (`/events/data`)

may throw 428 if no client connected

may throw 408 if no measurement was taken in time (insert the black filament)

Returns the stored black reference:

```json
{
    "r": 48.0,
    "g": 44.5,
    "b": 169.0
}
```
