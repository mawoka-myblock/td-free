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
        "threshold": 0.9 // between 0.999 and 0.001
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

### POST /ota/upload Flash new firmware

Uploads a raw firmware binary. The image is streamed directly into the unused OTA
partition, the next OTA slot is selected, and its state is set to pending verify.
On success the device reboots automatically after ~3 seconds.

The request must include a `Content-Length` header and the body must be the
firmware binary (e.g. produced by `espflash save-image` or `cargo run`).

Example using `curl`:

```bash
curl -X POST -H "Content-Type: application/octet-stream" \
  --data-binary @firmware.bin \
  http://10.10.10.1/ota/upload
```

Response on success:

```
OTA upload complete, rebooting...
```

Possible non-200 responses:

- `405` – only `POST` is supported.
- `423` – an OTA upload is already in progress.
- `503` – flash is currently busy (e.g. NVS write in progress).
- `500` / `413` – initialization, partition, or image size error.

> **Caution:** During an upload the device rejects concurrent config writes so
> that no two tasks access the flash peripheral simultaneously.
