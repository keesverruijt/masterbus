# Hardware

To talk to a MasterBus network from this project you need to bridge a
CAN port into your computer. There are two practical paths.

## Path A — Mastervolt USB Interface (works on Linux / macOS / Windows)

Mastervolt sells a USB-to-MasterBus adapter (vendor article number
77030200, retails around €200). It enumerates as a class-compliant HID
device — no vendor driver needed — and the cable terminates in the
standard MasterBus RJ45.

Plug it in, run `masterbus-tui` (or `masterbus-signalk`) without any
`can0`-style argument and it'll find the device. On macOS and Windows
this is the *only* option.

Pros: no firmware tinkering, plug-and-play, vendor-supported.
Cons: cost; one master per host (the USB driver doesn't share).

## Path B — Linux host + cheap CAN adapter (recommended)

Any Linux machine with a SocketCAN-capable CAN interface works. The
crate uses the kernel's standard `can0` (or similar) network device.
Bring it up at **250 kbit/s** before running anything:

    sudo ip link set can0 type can bitrate 250000
    sudo ip link set up can0

Confirm with `ip -det link show can0` (state should be `UP`).

### CANable / CANable Pro (USB stick, ~€25)

The easiest small-form-factor option. The CANable is an open-hardware
USB-to-CAN dongle (https://canable.io). Buy the *Pro* version if you
can — it has galvanic isolation, which matters on a powered MasterBus
where ground loops can cause noise or even hardware damage.

Out of the box CANable ships with `slcan` firmware (serial-tty mode),
which works but is slower and clunkier than `candleLight`. Reflash to
`candleLight`:

  1. Build or download `candleLight_fw` from
     https://github.com/candle-usb/candleLight_fw — the README walks
     through both prebuilt and source paths.
  2. Hold the CANable's `BOOT` button (or short the BOOT pads on the
     bare-PCB model) while plugging it in; it enumerates as a DFU
     device.
  3. `dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D candleLight_fw.bin`
  4. Unplug, plug back in. The kernel's `gs_usb` driver picks it up
     automatically and you get a `can0` interface.

### Raspberry Pi HATs

For a permanently-installed boat install, a CAN HAT on a Pi is tidy:

- **PiCAN2 / PiCAN3** (Copperhill) — proven, well-documented, isolated.
- **Waveshare RS485 CAN HAT** — cheap, works fine but check isolation.
- **MCP2515-based DIY HAT** — works if you can edit `/boot/config.txt`
  to enable the SPI overlay (`dtoverlay=mcp2515-can0,oscillator=16000000,interrupt=25`)
  and bring up `can0` at 250 kbit/s.

### Physical connection to the bus

MasterBus uses RJ45 connectors (8P8C, standard Ethernet plug) on twisted
pair. Pinout (looking into the device socket, contacts down):

    pin 1: NC
    pin 2: NC
    pin 3: CAN-L
    pin 4: +12 V DC (bus power, ~80 mA available)
    pin 5: GND
    pin 6: CAN-H
    pin 7: NC
    pin 8: NC

Make sure your adapter's CAN-H/CAN-L map to pins 6/3 respectively. A
DB9-to-RJ45 adapter cable is the usual route for adapters that come
with a DB9 connector.

The bus must be terminated with 120 Ω at each end. Most Mastervolt
devices include termination jumpers; the spec sheets show which devices
are typically at the ends. Improper termination causes intermittent
frame loss that looks exactly like a flaky adapter.

## Verifying the bus

Once `can0` is up, sniff with `cansniffer` or `candump`:

    candump -tA can0

You should see a steady stream of frames within seconds — every alive
device broadcasts a class-`0x04` status frame every ~1 second. If
`candump` is silent, check:

- Bitrate (250 kbit/s, not 125 or 500).
- Termination (120 Ω at *both* ends, not just one).
- CAN-H / CAN-L not swapped.
- For HATs: kernel module loaded, `dmesg | grep -i can`.

Then `masterbus-tui can0` should show all devices within a few seconds.
