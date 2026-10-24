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
    "led_brightness": 100, // status LED brightness in %, the status LED is always off during measurements
    "algo": {
        "b": 0.0,
        "m": 1.0,
        "threshold": 0.9 // between 0.999 and 0.001
    }
}
```

### GET/POST /config/rgb Set/Get RGB Multipliers
No longer used for the color, which comes from the color references below. Kept so that older
clients and stored settings keep working.

Sets/Gets rgb multipliers:
```json
{
    "red": 1.0,
    "green": 1.0,
    "blue": 1.0,
    "brightness": 1.0,
    "td_reference": 0.0,
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

### Color references
The color is corrected with filaments of known color. With no reference the color comes from the
white balance only, one or two references correct the white point, three or more fit a full 3x3
color matrix. At most 8 references are stored.

All endpoints return the list of references:

```json
[
    {
        "target": [232, 230, 208],
        "measured": [236, 222, 255],
        "corrected": [221, 232, 223]
    }
]
```

`target` is the color of the spool, `measured` the color with the white balance only, `corrected` what
the device shows with the correction from all references.

#### GET /config/color-references
Returns the references.

#### POST /config/color-references/add
Adds the inserted filament as a reference. Needs a client listening to server sent events
(`/events/data`) and a filament in the chamber.

may throw 428 if no client connected

may throw 408 if no filament is inserted

may throw 409 if all 8 references are used

```json
{
    "target_r": 232,
    "target_g": 230,
    "target_b": 208
}
```

`POST /config/auto-calibrate` is the older name of this endpoint and does the same.

#### POST /config/color-references/remove
```json
{
    "index": 0
}
```

#### POST /config/color-references/clear
Removes all references.
