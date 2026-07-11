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
    "buf_count": Option<12>
}
```

### GET/POST /config/settings Set/Get Config
Sets/Gets settings:

```json
{
    "algo": {
        "b": 0.0,
        "m": 1.0,
        "threshold": 0.9 // between 0.999 and 0.001
    }
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
    "has_color": false,
    "version": "06556+4-3.3"
}
```
