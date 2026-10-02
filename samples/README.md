# Sample bus dumps

Real buses, captured with `masterbus-dump --menus all --values all`, for
reading what devices out there report without having them on your desk.
The format is the one `masterbus-dump` writes (`"format": 1`), with the
mapping that was in use at the time embedded under `mapping`.

Dumps are anonymised before they land here: the boat's name is removed, and
every serial number keeps its first five characters (production code) with
the last four digits replaced, consistently everywhere the serial occurs —
the device record, the mapping's device keys, and fields that echo a serial,
such as a DC distributor's `Set Serial`. Bus addresses, articles, firmware
and values are as captured.

| File | Captured | Devices | Notes |
|------|----------|---------|-------|
| `mcu-czone-19-devices.json` | 2026-10-01, masterbus-dump 0.4.2 | 19 | MCU charger/inverter/solar, 5× INT (one a CZone bridge), 4× DCD, 2× DSD, DSI, 2× MAC, ISO, MSH, BAT, DIS; mapping of 83 fields on 17 devices |
