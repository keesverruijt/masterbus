# MasterBus fields of a 19-device bus (MCU, CZone bridge)

Every field the `masterbus-signalk` daemon 0.6.0-beta.2 lists for the 19 devices of the bus in `mcu-czone-19-devices.json`, all menus
(monitoring, alarm, history, configuration), read through its API (`GET /api/devices/{serial}?menu=...`) on 2026-11-19.
Same anonymisation as the dump: the boat name removed, serials as in `mcu-czone-19-devices.json`. Values as read at that moment.

"Signal K path" is the mapping in use on this boat (404 fields, 26 with `put`). Configuration fields are not mapped there
(the owner reads them once a day through the API); commands and event outputs are not mapped on purpose.

"Access": read; read/write (writable at user level); event output (exists only as an event source or target, no value);
command / event output (a writable trigger such as Reset alarms or Disabled).

| Device | Article | Firmware | Fields | Mapped |
|---|---|---|--:|--:|
| MCU Chg/Inv/Sol | 38013000 | 2.67 | 149 | 49 |
| INT 5 Boiler | 77031500 | 1.6 | 23 | 2 |
| INT CZone-MB | 77033000 | 0.1 | 100 | 81 |
| INT 3 Computer | 77030500 | 1.13 | 11 | 2 |
| INT 1 DCpower | 77030500 | 1.13 | 15 | 2 |
| INT 4 SolarDeck | 77030500 | 1.13 | 15 | 2 |
| DSD 1 Power | 77020400 | 2.22 | 144 | 44 |
| DSI 1 Input4x | 77030900 | 2.4 | 62 | 1 |
| DCD 4 START ALL | 77020200 | 2.56 | 34 | 6 |
| DCD 2 SERVICE CH | 77020200 | 2.56 | 34 | 6 |
| DCD 3 VITAL ALL | 77020200 | 2.56 | 34 | 6 |
| DCD 1 SERVICE LD | 77020200 | 2.56 | 34 | 6 |
| DSD 2 Switch | 77020400 | 2.22 | 141 | 42 |
| BAT SERVICE | 66016000 | 2.15 | 83 | 17 |
| DIS | 77010310 | 2.14 | 116 | 7 |
| MAC 1 BOW | 81205100 | 1.8 | 70 | 21 |
| MAC 3 VITAL | 81205105 | 1.8 | 71 | 21 |
| ISO GI Trafo | 88000355 | 1.7 | 42 | 36 |
| MSH VITAL | 77020110 | 0.66 | 76 | 32 |

## MCU Chg/Inv/Sol

Article 38013000, firmware 2.67, serial KO17R2637, Signal K instance `chgInvSol`, menus: monitoring, alarm, history, configuration.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x010 | General | Device state | monitoring | read |  | Charging | Standby, Inverting, Alarm, Charging, Supporting, Overload, Low Bat | `electrical.inverters.chgInvSol.inverterMode` |
| 0x013 | General | Mains fuse | monitoring | read/write | A | 8.0 |  | `electrical.chargers.chgInvSol.acin.currentLimit` (put) |
| 0x014 | General | Inverter | monitoring | read/write |  | Off | Off, On | `electrical.inverters.chgInvSol.enabled` (put) |
| 0x03C | General | User mode | monitoring | read |  | On | Initializing, Disabled, StartFromBatt, StartFromAC, ManualMode, On, Auto Reset, Error, Shutdown | `electrical.inverters.chgInvSol.userMode` |
| 0x004 | General | AC in state | monitoring | read |  | Mains | No AC present, Generator, Mains, Poor quality | `electrical.chargers.chgInvSol.acin.state` |
| 0x005 | General | AC out state | monitoring | read |  | AC out 1 & 2 | No AC output, AC out 1 only, AC out 1 & 2 | `electrical.inverters.chgInvSol.ac.state` |
| 0x03A | General | Main charger | monitoring | read |  | On | Off, On | `electrical.chargers.chgInvSol.enabled` |
| 0x00E | General | Sec. charger | monitoring | read/write |  | On | Off, On | `electrical.chargers.chgInvSolSec.enabled` (put) |
| 0x00F | General | Solar charger | monitoring | read/write |  | On | Off, On | `electrical.solar.chgInvSol.enabled` (put) |
| 0x012 | Battery (DC) | Main charger | monitoring | read |  | Float | Standby, Bulk, Absorption, Float, Constant voltage, Equalize | `electrical.chargers.chgInvSol.chargingMode` |
| 0x006 | Battery (DC) | Main battery | monitoring | read | V | 13.500233 |  | `electrical.inverters.chgInvSol.dc.voltage` |
| 0x007 | Battery (DC) | Main battery | monitoring | read | A | 7.608772 |  | `electrical.inverters.chgInvSol.dc.current` |
| 0x015 | Battery (DC) | Main battery | monitoring | read | °C | 9.0 |  | `electrical.inverters.chgInvSol.dc.temperature` |
| 0x02E | Sec. charger | Sec. charger | monitoring | read |  | On | Off, Bulk, Absorption, Float, Constant voltage, Standby, Error, On | `electrical.chargers.chgInvSolSec.chargingMode` |
| 0x02B | Sec. charger | Sec. battery | monitoring | read | V | 13.4498 |  | `electrical.chargers.chgInvSolSec.voltage` |
| 0x02C | Sec. charger | Sec. battery | monitoring | read | A | 0.25223336 |  | `electrical.chargers.chgInvSolSec.current` |
| 0x008 | AC inputs | Mains | monitoring | read | V | 230.21483 |  | `electrical.chargers.chgInvSol.acin.voltage` |
| 0x009 | AC inputs | Mains | monitoring | read | A | 4.8039575 |  | `electrical.chargers.chgInvSol.acin.current` |
| 0x05E | AC inputs | Mains | monitoring | read | W | 1099.1646 |  | `electrical.chargers.chgInvSol.acin.power` |
| 0x05F | AC inputs | Generator | monitoring | read | V | – |  | – |
| 0x060 | AC inputs | Generator | monitoring | read | A | – |  | – |
| 0x061 | AC inputs | Generator | monitoring | read | W | – |  | – |
| 0x00A | AC outputs | AC Output 1 | monitoring | read | V | 230.20537 |  | `electrical.inverters.chgInvSol.ac.voltage` |
| 0x00B | AC outputs | AC Output 1 | monitoring | read | A | 4.37072 |  | `electrical.inverters.chgInvSol.ac.current` |
| 0x062 | AC outputs | AC Output 1 | monitoring | read | W | 1002.43976 |  | `electrical.inverters.chgInvSol.ac.power` |
| 0x063 | AC outputs | AC Output 2 | monitoring | read | V | 230.20363 |  | `electrical.inverters.chgInvSol.ac2.voltage` |
| 0x064 | AC outputs | AC Output 2 | monitoring | read | A | 0.058008514 |  | `electrical.inverters.chgInvSol.ac2.current` |
| 0x065 | AC outputs | AC Output 2 | monitoring | read | W | -3.5104656 |  | `electrical.inverters.chgInvSol.ac2.power` |
| 0x02A | Solar Input | State | monitoring | read |  | Float | Off, Standby, Charging, Bulk, Absorption, Float | `electrical.solar.chgInvSol.chargingMode` |
| 0x029 | Solar Input | Solar Input | monitoring | read | W | 32.387287 |  | `electrical.solar.chgInvSol.panelPower` |
| 0x205 | Alarms | Overload | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.overload` |
| 0x200 | Alarms | Over temperature | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.overTemperature` |
| 0x201 | Alarms | Battery low | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.batteryLow` |
| 0x202 | Alarms | Battery high | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.batteryHigh` |
| 0x203 | Alarms | Batt temperature | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.battTemperature` |
| 0x204 | Alarms | Temp sense error | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.tempSenseError` |
| 0x206 | Alarms | Sync. Error | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.syncError` |
| 0x207 | Alarms | Config error | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.configError` |
| 0x208 | Alarms | Install error | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.installError` |
| 0x209 | Alarms | System error | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.systemError` |
| 0x20A | Alarms | Cable losses | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.cableLosses` |
| 0x20B | Alarms | Shunt mismatch | alarm | read |  | 0 |  | `electrical.inverters.chgInvSol.alarms.shuntMismatch` |
| 0x303 | Battery (DC) | Highest vlt main | history | read | V | 16.605543 |  | `electrical.inverters.chgInvSol.batteryDc.highestVltMain.voltage` |
| 0x305 | Battery (DC) | Highest volt 2nd | history | read | V | 16.40431 |  | `electrical.inverters.chgInvSol.batteryDc.highestVolt2nd.voltage` |
| 0x306 | Battery (DC) | Lowest volt 2nd | history | read | V | 11.990869 |  | `electrical.inverters.chgInvSol.batteryDc.lowestVolt2nd.voltage` |
| 0x307 | Battery (DC) | Cycles completed | history | read |  | 129.0 |  | `electrical.inverters.chgInvSol.batteryDc.cyclesCompleted` |
| 0x308 | Battery (DC) | Cycles Aborted | history | read |  | 3.0 |  | `electrical.inverters.chgInvSol.batteryDc.cyclesAborted` |
| 0x30C | Solar Input | Energy | history | read | kWh | 2.068 |  | `electrical.inverters.chgInvSol.solarInput.energy` |
| 0x30A | Mains Input | Energy | history | read | kWh | 14759.819 |  | `electrical.inverters.chgInvSol.mainsInput.energy` |
| 0x30B | Generator input | Energy | history | read | kWh | 0.0 |  | `electrical.inverters.chgInvSol.generatorInput.energy` |
| 0x30D | AC Output 1 | Energy | history | read | kWh | 9816.2 |  | `electrical.inverters.chgInvSol.acOutput1.energy` |
| 0x30E | AC Output 2 | Energy | history | read | kWh | 1230.7 |  | `electrical.inverters.chgInvSol.acOutput2.energy` |
| 0x085 | Outputs | Inverter | monitoring | event output |  | – |  | – |
| 0x087 | Outputs | Charger | monitoring | event output |  | – |  | – |
| 0x07F | Outputs | Bulk | monitoring | event output |  | – |  | – |
| 0x081 | Outputs | Absorption | monitoring | event output |  | – |  | – |
| 0x083 | Outputs | Float | monitoring | event output |  | – |  | – |
| 0x089 | Outputs | Mains limit A | monitoring | event output |  | – |  | – |
| 0x08B | Outputs | Mains limit B | monitoring | event output |  | – |  | – |
| 0x08D | Outputs | Mains limit C | monitoring | event output |  | – |  | – |
| 0x08F | Outputs | ACout 2 Enabled | monitoring | event output |  | – |  | – |
| 0x091 | Outputs | Force Sec Charge | monitoring | event output |  | – |  | – |
| 0x099 | Outputs | Power Off | monitoring | event output |  | – |  | – |
| 0x097 | Outputs | Silent Mode | monitoring | event output |  | – |  | – |
| 0x09B | Outputs | Gen limit A | monitoring | event output |  | – |  | – |
| 0x09D | Outputs | Gen limit B | monitoring | event output |  | – |  | – |
| 0x09F | Outputs | Gen limit C | monitoring | event output |  | – |  | – |
| 0x0AF | Outputs | Mains limit D | monitoring | event output |  | – |  | – |
| 0x0B1 | Outputs | Mains limit E | monitoring | event output |  | – |  | – |
| 0x0B3 | Outputs | Mains limit F | monitoring | event output |  | – |  | – |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano | – |
| 0x104 | Device | Name | configuration | read/write |  | Chg/Inv/Sol |  | – |
| 0x189 | Device | Lock config. | configuration | read |  | 0 |  | – |
| 0x103 | System | MasterBus power | configuration | read/write |  | 0 |  | – |
| 0x198 | System | Silent Mode | configuration | read/write |  | False |  | – |
| 0x197 | System | Silent Mode | configuration | read/write | % | 36.0 |  | – |
| 0x118 | System | Shunt device | configuration | read/write |  | 0 |  | – |
| 0x119 | System | Abs. return amp. | configuration | read | A | 9.2 |  | – |
| 0x11A | System | Deselect shunt | configuration | read/write |  | False |  | – |
| 0x12A | Main charger | Maximum current | configuration | read/write | A | 150.0 |  | – |
| 0x12C | Main charger | Method | configuration | read/write |  | 3-Step+ (IUoU) | 3-Step+ (IUoU), Constant voltage | – |
| 0x12B | Main charger | Battery type | configuration | read/write |  | MLI | User defined, Flooded, GEL, AGM, Spiral, MLI, Flooded traction, Nickel Cadmium | – |
| 0x11B | Bulk | Bulk voltage | configuration | read | V | 14.25 |  | – |
| 0x11E | Bulk | Min bulk time | configuration | read | min | 2.0 |  | – |
| 0x11F | Bulk | Start bulk time | configuration | read | V | 13.25 |  | – |
| 0x121 | Bulk | Max bulk time | configuration | read | min | 480.0 |  | – |
| 0x120 | Bulk | Bulk ret. volt. | configuration | read | V | 13.25 |  | – |
| 0x122 | Bulk | Bulk return time | configuration | read | sec | 240.0 |  | – |
| 0x11C | Absorption | Abs. voltage | configuration | read | V | 14.25 |  | – |
| 0x123 | Absorption | Max absorp. time | configuration | read | min | 240.0 |  | – |
| 0x125 | Absorption | Return amps | configuration | read | A | 6.0 |  | – |
| 0x124 | Absorption | Min absorp. time | configuration | read | min | 15.0 |  | – |
| 0x11D | Float | Float voltage | configuration | read | V | 13.5 |  | – |
| 0x154 | Sec. charger | Method | configuration | read/write |  | On | Follow main, Events only, On | – |
| 0x107 | Inverter | Voltage | configuration | read/write | V | 230.0 |  | – |
| 0x105 | Inverter | Frequency | configuration | read/write | Hz | 50.0 |  | – |
| 0x106 | Inverter | Enable GND rel. | configuration | read/write |  | 0 |  | – |
| 0x108 | Inverter | Energy save mode | configuration | read/write |  | 0 |  | – |
| 0x109 | Inverter | Scanning below | configuration | read/write | W | 20.0 |  | – |
| 0x10C | Inverter | Dynamic window | configuration | read/write |  | Use battery type | Use battery type, User defined | – |
| 0x10A | Inverter | DC high off | configuration | read | V | 16.0 |  | – |
| 0x10B | Inverter | DC high on | configuration | read | V | 14.5 |  | – |
| 0x10D | Inverter | DC low off I=0% | configuration | read | V | 10.5 |  | – |
| 0x10E | Inverter | DC low off I=2% | configuration | read | V | 10.5 |  | – |
| 0x10F | Inverter | DC low off 100% | configuration | read | V | 10.5 |  | – |
| 0x110 | Inverter | DC low on | configuration | read | V | 12.0 |  | – |
| 0x111 | Inverter | DC low off delay | configuration | read | sec | 30.0 |  | – |
| 0x12D | Transfer | AC input policy | configuration | read/write |  | Mains preferred | Mains preferred, Gen. preferent | – |
| 0x12E | Transfer | AC out2 policy | configuration | read/write |  | Always on | Events only, Generator input, Mains Input, Gen/mains input, Always on | – |
| 0x117 | Transfer | Gen. fuse max. | configuration | read/write | A | 25.0 |  | – |
| 0x19E | Transfer | Gen limit A | configuration | read/write | A | 6.0 |  | – |
| 0x19F | Transfer | Gen limit B | configuration | read/write | A | 10.0 |  | – |
| 0x1A0 | Transfer | Gen limit C | configuration | read/write | A | 16.0 |  | – |
| 0x185 | Transfer | Mains fuse max. | configuration | read/write | A | 30.0 |  | – |
| 0x186 | Transfer | Mains limit A | configuration | read/write | A | 6.0 |  | – |
| 0x187 | Transfer | Mains limit B | configuration | read/write | A | 10.0 |  | – |
| 0x188 | Transfer | Mains limit C | configuration | read/write | A | 16.0 |  | – |
| 0x1AA | Transfer | Mains limit D | configuration | read/write | A | 20.0 |  | – |
| 0x1AB | Transfer | Mains limit E | configuration | read/write | A | 25.0 |  | – |
| 0x1AC | Transfer | Mains limit F | configuration | read/write | A | 30.0 |  | – |
| 0x12F | Transfer | Power sharing | configuration | read/write |  | 0 |  | – |
| 0x130 | Transfer | Gen/Main support | configuration | read/write |  | 0 |  | – |
| 0x131 | AC input limits | Gen. high volt. | configuration | read/write | V | 260.0 |  | – |
| 0x132 | AC input limits | Gen. low volt. | configuration | read/write | V | 180.0 |  | – |
| 0x18E | AC input limits | Gen. freq. range | configuration | read/write |  | 0.0 |  | – |
| 0x133 | AC input limits | Gen. high freq. | configuration | read/write | Hz | 58.0 |  | – |
| 0x134 | AC input limits | Gen. low freq. | configuration | read/write | Hz | 40.0 |  | – |
| 0x135 | AC input limits | Generator delay | configuration | read/write | sec | 10.0 |  | – |
| 0x112 | AC input limits | Mains high volt. | configuration | read/write | V | 260.0 |  | – |
| 0x113 | AC input limits | Mains low volt. | configuration | read/write | V | 180.0 |  | – |
| 0x114 | AC input limits | Mains high freq. | configuration | read/write | Hz | 65.0 |  | – |
| 0x115 | AC input limits | Mains low freq. | configuration | read/write | Hz | 40.0 |  | – |
| 0x116 | AC input limits | Mains delay | configuration | read/write | sec | 10.0 |  | – |
| 0x179 | DIP switches (A) | Parallel mode | configuration | read |  | Default | Default, Replica, Primary | – |
| 0x17A | DIP switches (A) | Inverter freq. | configuration | read |  | Default | Default, 60 Hz | – |
| 0x17B | DIP switches (A) | Energy save mode | configuration | read |  | Scan mode | Default, Scan mode | – |
| 0x17C | DIP switches (A) | Sec. charger | configuration | read |  | Default | Default, 12V 3-Step+, Constant voltage, Constant voltage | – |
| 0x17D | DIP switches (A) | Main battery | configuration | read |  | Default | Default, AGM, GEL, Flooded traction | – |
| 0x17E | DIP switches (B) | Mains fuse max. | configuration | read |  | 16 A | Default, 10 A, 16 A, 6 A | – |
| 0x17F | DIP switches (B) | Gen. fuse max. | configuration | read |  | Default | Default, 50 A | – |
| 0x180 | DIP switches (B) | Power sharing | configuration | read |  | Default | Default, Disabled | – |
| 0x181 | DIP switches (B) | Gen/Main support | configuration | read |  | Default | Default, Enabled | – |
| 0x182 | DIP switches (B) | AC volt in range | configuration | read |  | Default | Default, Narrow | – |
| 0x183 | DIP switches (B) | Gen. freq. range | configuration | read |  | Default | Default, Narrow | – |
| 0x184 | DIP switches (B) | Ground relay | configuration | read |  | Enabled | Default, Enabled | – |
| 0x1A1 | Processor 1 | Version | configuration | read |  | 1.34.0 |  | – |
| 0x1A2 | Processor 1 | Revision | configuration | read |  | AA0134 |  | – |
| 0x178 | Processor 2 | Version | configuration | read |  | 1.33 |  | – |
| 0x1AD | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, Inverting, Charging, Supporting, Overload, Low Bat, Alarm, Generator input, Mains Input, ACout 2 Enabled, Sec charging, Solar charging, Bulk, Absorption, Float, External Fan, Silent Mode, Gen limit A, Gen limit B, Gen limit C, Mains limit A, Mains limit B, Mains limit C, Mains limit D, Mains limit E, Mains limit F | – |

## INT 5 Boiler

Article 77031500, firmware 1.6, serial KN14G2318, Signal K instance `5Boiler`, menus: monitoring, alarm, history, configuration.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | Standby | Standby, Activated | `electrical.switches.5Boiler.state` |
| 0x001 | Device | Control | monitoring | read/write |  | False |  | `electrical.switches.5Boiler.state` (put) |
| 0x002 | Outputs | Activate | monitoring | event output |  | – |  | – |
| 0x100 | General | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x101 | General | Device name | configuration | read/write |  | 5 Boiler |  | – |
| 0x102 | General | Events locked | configuration | read/write |  | 0 |  | – |
| 0x103 | Timing | Min. runtime | configuration | read/write |  | 0 |  | – |
| 0x104 | Timing | Min. runtime | configuration | read/write | Minutes | 30.0 |  | – |
| 0x105 | Timing | Max. runtime | configuration | read/write |  | 0 |  | – |
| 0x106 | Timing | Max. runtime | configuration | read/write | Minutes | 45.0 |  | – |
| 0x10F | Events | Event 1 source | configuration | read/write |  | State | Disabled, State | – |
| 0x110 | Events | Event 1 target | configuration | read/write |  | 0 |  | – |
| 0x111 | Events | Event 1 command | configuration | read/write |  | 2 |  | – |
| 0x112 | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x113 | Events | Event 2 source | configuration | read/write |  | State | Disabled, State | – |
| 0x114 | Events | Event 2 target | configuration | read/write |  | 1 |  | – |
| 0x115 | Events | Event 2 command | configuration | read/write |  | 16 |  | – |
| 0x116 | Events | Event 2 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x117 | Events | Event 3 source | configuration | read/write |  | State | Disabled, State | – |
| 0x118 | Events | Event 3 target | configuration | read/write |  | 1 |  | – |
| 0x119 | Events | Event 3 command | configuration | read/write |  | 0 |  | – |
| 0x11A | Events | Event 3 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x11B | Events | Event 4 source | configuration | read/write |  | Disabled | Disabled, State | – |

## INT CZone-MB

Article 77033000, firmware 0.1, serial B822A8869, Signal K instance `cZoneMb`, menus: monitoring, alarm, configuration, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x003 |  | General | monitoring | read |  | OK | OK | `electrical.switches.cZoneMb.deviceState` |
| 0x024 | Control to CZ | Control to CZ01 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz01` |
| 0x026 | Control to CZ | Control to CZ02 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz02` |
| 0x028 | Control to CZ | Control to CZ03 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz03` |
| 0x02A | Control to CZ | Control to CZ04 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz04` |
| 0x02C | Control to CZ | Control to CZ05 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz05` |
| 0x02E | Control to CZ | Control to CZ06 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz06` |
| 0x030 | Control to CZ | Control to CZ07 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz07` |
| 0x032 | Control to CZ | Control to CZ08 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz08` |
| 0x034 | Control to CZ | Control to CZ09 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz09` |
| 0x036 | Control to CZ | Control to CZ10 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz10` |
| 0x038 | Control to CZ | Control to CZ11 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz11` |
| 0x03A | Control to CZ | Control to CZ12 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz12` |
| 0x03C | Control to CZ | Control to CZ13 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz13` |
| 0x03E | Control to CZ | Control to CZ14 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz14` |
| 0x040 | Control to CZ | Control to CZ15 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz15` |
| 0x042 | Control to CZ | Control to CZ16 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.controlToCz.controlToCz16` |
| 0x014 | Feedback to MB | Feedback to MB01 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb01` |
| 0x015 | Feedback to MB | Feedback to MB02 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb02` |
| 0x016 | Feedback to MB | Feedback to MB03 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb03` |
| 0x017 | Feedback to MB | Feedback to MB04 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb04` |
| 0x018 | Feedback to MB | Feedback to MB05 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb05` |
| 0x019 | Feedback to MB | Feedback to MB06 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb06` |
| 0x01A | Feedback to MB | Feedback to MB07 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb07` |
| 0x01B | Feedback to MB | Feedback to MB08 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb08` |
| 0x01C | Feedback to MB | Feedback to MB09 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb09` |
| 0x01D | Feedback to MB | Feedback to MB10 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb10` |
| 0x01E | Feedback to MB | Feedback to MB11 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb11` |
| 0x01F | Feedback to MB | Feedback to MB12 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb12` |
| 0x020 | Feedback to MB | Feedback to MB13 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb13` |
| 0x021 | Feedback to MB | Feedback to MB14 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb14` |
| 0x022 | Feedback to MB | Feedback to MB15 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb15` |
| 0x023 | Feedback to MB | Feedback to MB16 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToMb.feedbackToMb16` |
| 0x004 | Control to MB | Control to MB01 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb01` |
| 0x005 | Control to MB | Control to MB02 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb02` |
| 0x006 | Control to MB | Control to MB03 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb03` |
| 0x007 | Control to MB | Control to MB04 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb04` |
| 0x008 | Control to MB | Control to MB05 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb05` |
| 0x009 | Control to MB | Control to MB06 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb06` |
| 0x00A | Control to MB | Control to MB07 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb07` |
| 0x00B | Control to MB | Control to MB08 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb08` |
| 0x00C | Control to MB | Control to MB09 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb09` |
| 0x00D | Control to MB | Control to MB10 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb10` |
| 0x00E | Control to MB | Control to MB11 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb11` |
| 0x00F | Control to MB | Control to MB12 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb12` |
| 0x010 | Control to MB | Control to MB13 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb13` |
| 0x011 | Control to MB | Control to MB14 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb14` |
| 0x012 | Control to MB | Control to MB15 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb15` |
| 0x013 | Control to MB | Control to MB16 | monitoring | read/write |  | 0.0 |  | `electrical.switches.cZoneMb.controlToMb.controlToMb16` |
| 0x044 | Feedback to CZ | Feedback to CZ01 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz01` |
| 0x046 | Feedback to CZ | Feedback to CZ02 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz02` |
| 0x048 | Feedback to CZ | Feedback to CZ03 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz03` |
| 0x04A | Feedback to CZ | Feedback to CZ04 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz04` |
| 0x04C | Feedback to CZ | Feedback to CZ05 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz05` |
| 0x04E | Feedback to CZ | Feedback to CZ06 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz06` |
| 0x050 | Feedback to CZ | Feedback to CZ07 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz07` |
| 0x052 | Feedback to CZ | Feedback to CZ08 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz08` |
| 0x054 | Feedback to CZ | Feedback to CZ09 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz09` |
| 0x056 | Feedback to CZ | Feedback to CZ10 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz10` |
| 0x058 | Feedback to CZ | Feedback to CZ11 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz11` |
| 0x05A | Feedback to CZ | Feedback to CZ12 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz12` |
| 0x05C | Feedback to CZ | Feedback to CZ13 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz13` |
| 0x05E | Feedback to CZ | Feedback to CZ14 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz14` |
| 0x060 | Feedback to CZ | Feedback to CZ15 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz15` |
| 0x062 | Feedback to CZ | Feedback to CZ16 | monitoring | event output |  | 0.0 |  | `electrical.switches.cZoneMb.feedbackToCz.feedbackToCz16` |
| 0x200 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm` |
| 0x201 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x201` |
| 0x202 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x202` |
| 0x203 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x203` |
| 0x204 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x204` |
| 0x205 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x205` |
| 0x206 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x206` |
| 0x207 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x207` |
| 0x208 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x208` |
| 0x209 | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x209` |
| 0x20A | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x20A` |
| 0x20B | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x20B` |
| 0x20C | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x20C` |
| 0x20D | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x20D` |
| 0x20E | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x20E` |
| 0x20F | Alarm | Alarm | alarm | read |  | None | None, AC Voltage error, AC Freq. error, AC High power, DC Low voltage, DC Very low volt, DC High Voltage, DC Low SOC, High level, Low level, Out of range, Low run current, Over current, Short circuit, Missing cmndr., Reverse current, Calibration err., Missing output, On, AC very high pwr, AC low power, DC very low SOC, DC high SOC, DC load shed low, DC shed very low, AC load shed low, AC shed very low, Reverse polarity, Manual Override, MasterBus Alarm, Hardware Fault, Loss of Supply, Very High Level, Very Low Level, PGN Switch | `electrical.switches.cZoneMb.alarm.alarm0x20F` |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | Device | Name | configuration | read/write |  | CZone-MB |  | – |
| 0x105 | Device | Factory settings | configuration | read |  | False |  | – |
| 0x106 | Device | Handle CZ Alarms | configuration | read/write |  | 0 |  | – |
| 0x107 | Device | CZone Dipswitch: | configuration | read |  | 11111111 (3) |  | – |
| 0x108 | Metering 1 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x11A | Metering 2 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x12C | Metering 3 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x13E | Metering 4 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x150 | Metering 5 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x162 | Metering 6 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x174 | Metering 7 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x186 | Metering 8 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x198 | Metering 9 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x1AA | Metering 10 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x1BC | Metering 11 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x1CE | Metering 12 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x1E0 | Metering 13 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |
| 0x1F2 | Metering 14 | Type | configuration | read/write |  | None | None, AC, DC, Fluidlevel, Inv/Chg Status | – |

## INT 3 Computer

Article 77030500, firmware 1.13, serial L820I6069, Signal K instance `3Computer`, menus: monitoring, configuration, alarm, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | Standby | Standby, Activated | `electrical.switches.3Computer.state` |
| 0x001 | Device | Override | monitoring | read/write |  | Standby | Standby, Activated | `electrical.switches.3Computer.state` (put) |
| 0x002 | Outputs | Activate | monitoring | event output |  | – | Off, On | – |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x101 | Device | Device name | configuration | read/write |  | 3 Computer |  | – |
| 0x102 | Timing | Pulse (1 sec) | configuration | read/write |  | 0 |  | – |
| 0x103 | Timing | Min. runtime | configuration | read/write |  | 0 |  | – |
| 0x104 | Timing | Min. runtime | configuration | read/write | min | 15.0 |  | – |
| 0x105 | Timing | Max. runtime | configuration | read/write |  | 0 |  | – |
| 0x106 | Timing | Max. runtime | configuration | read/write | min | 480.0 |  | – |
| 0x10C | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, State | – |

## INT 1 DCpower

Article 77030500, firmware 1.13, serial L820I6649, Signal K instance `1DCpower`, menus: monitoring, configuration, alarm, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | Standby | Standby, Activated | `electrical.switches.1DCpower.state` |
| 0x001 | Device | Override | monitoring | read/write |  | Standby | Standby, Activated | `electrical.switches.1DCpower.state` (put) |
| 0x002 | Outputs | Activate | monitoring | event output |  | – | Off, On | – |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x101 | Device | Device name | configuration | read/write |  | 1 DCpower |  | – |
| 0x102 | Timing | Pulse (1 sec) | configuration | read/write |  | 0 |  | – |
| 0x103 | Timing | Min. runtime | configuration | read/write |  | 0 |  | – |
| 0x104 | Timing | Min. runtime | configuration | read/write | min | 15.0 |  | – |
| 0x105 | Timing | Max. runtime | configuration | read/write |  | 0 |  | – |
| 0x106 | Timing | Max. runtime | configuration | read/write | min | 480.0 |  | – |
| 0x10C | Events | Event 1 source | configuration | read/write |  | State | Disabled, State | – |
| 0x10D | Events | Event 1 target | configuration | read/write |  | 9 |  | – |
| 0x10E | Events | Event 1 command | configuration | read/write |  | 5 |  | – |
| 0x10F | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x110 | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, State | – |

## INT 4 SolarDeck

Article 77030500, firmware 1.13, serial S619I5788, Signal K instance `4SolarDeck`, menus: monitoring, configuration, alarm, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | Activated | Standby, Activated | `electrical.switches.4SolarDeck.state` |
| 0x001 | Device | Override | monitoring | read/write |  | Activated | Standby, Activated | `electrical.switches.4SolarDeck.state` (put) |
| 0x002 | Outputs | Activate | monitoring | event output |  | – | Off, On | – |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x101 | Device | Device name | configuration | read/write |  | 4 SolarDeck |  | – |
| 0x102 | Timing | Pulse (1 sec) | configuration | read/write |  | 0 |  | – |
| 0x103 | Timing | Min. runtime | configuration | read/write |  | 0 |  | – |
| 0x104 | Timing | Min. runtime | configuration | read/write | min | 15.0 |  | – |
| 0x105 | Timing | Max. runtime | configuration | read/write |  | 0 |  | – |
| 0x106 | Timing | Max. runtime | configuration | read/write | min | 480.0 |  | – |
| 0x10C | Events | Event 1 source | configuration | read/write |  | State | Disabled, State | – |
| 0x10D | Events | Event 1 target | configuration | read/write |  | 9 |  | – |
| 0x10E | Events | Event 1 command | configuration | read/write |  | 3 |  | – |
| 0x10F | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x110 | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, State | – |

## DSD 1 Power

Article 77020400, firmware 2.22, serial HD03E1534, Signal K instance `1Power`, menus: monitoring, alarm, history, configuration.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x030 | General | Device state | monitoring | read |  | OK | Disabled, OK, Alarm | `electrical.switches.1Power.deviceState` |
| 0x003 | Channels | Bilge Pump Front | monitoring | event output |  | True |  | `electrical.switches.1Power.bilgePumpFront.state` |
| 0x005 | Channels | Bilge Pump Rear | monitoring | event output |  | True |  | `electrical.switches.1Power.bilgePumpRear.state` |
| 0x007 | Channels | Shower Pump | monitoring | event output |  | False |  | `electrical.switches.1Power.showerPump.state` (put) |
| 0x009 | Channels | Spare | monitoring | event output |  | False |  | `electrical.switches.1Power.spare.state` (put) |
| 0x00B | Channels | Switched Power | monitoring | event output |  | False |  | `electrical.switches.1Power.switchedPower.state` (put) |
| 0x00D | Channels | Constant Power | monitoring | event output |  | True |  | `electrical.switches.1Power.constantPower.state` (put) |
| 0x00F | Channels | Main H5 Pilot | monitoring | event output |  | False |  | `electrical.switches.1Power.mainH5Pilot.state` (put) |
| 0x011 | Channels | Backup H5 Pilot | monitoring | event output |  | False |  | `electrical.switches.1Power.backupH5Pilot.state` (put) |
| 0x013 | Channels | Freshwater pump | monitoring | event output |  | False |  | `electrical.switches.1Power.freshwaterPump.state` (put) |
| 0x015 | Channels | Spare | monitoring | event output |  | False |  | `electrical.switches.1Power.spare2.state` |
| 0x02E | Reset | Reset alarms | monitoring | event output |  | False |  | `electrical.switches.1Power.reset.resetAlarms` (put) |
| 0x21E | Alarm status | Bilge Pump Front | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.bilgePumpFront` |
| 0x21F | Alarm status | Bilge Pump Rear | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.bilgePumpRear` |
| 0x220 | Alarm status | Shower Pump | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.showerPump` |
| 0x221 | Alarm status | Spare | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.spare` |
| 0x222 | Alarm status | Switched Power | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.switchedPower` |
| 0x223 | Alarm status | Constant Power | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.constantPower` |
| 0x224 | Alarm status | Main H5 Pilot | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.mainH5Pilot` |
| 0x225 | Alarm status | Backup H5 Pilot | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.backupH5Pilot` |
| 0x226 | Alarm status | Freshwater pump | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.freshwaterPump` |
| 0x227 | Alarm status | Spare | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.1Power.alarmStatus.spare0x227` |
| 0x22A | System | Temperature | alarm | read |  | Normal | Normal, High, Critical | `electrical.switches.1Power.system.temperature` |
| 0x22B | System | Battery Low | alarm | read |  | 0 |  | `electrical.switches.1Power.system.batteryLow` |
| 0x017 |  | Channel 1 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel1` |
| 0x018 |  | Channel 2 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel2` |
| 0x019 |  | Channel 3 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel3` |
| 0x01A |  | Channel 4 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel4` |
| 0x01B |  | Channel 5 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel5` |
| 0x01C |  | Channel 6 | monitoring | read | A(rms) | 0.019022623 |  | `electrical.switches.1Power.current.channel6` |
| 0x01D |  | Channel 7 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel7` |
| 0x01E |  | Channel 8 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel8` |
| 0x01F |  | Channel 9 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel9` |
| 0x020 |  | Channel 10 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.1Power.current.channel10` |
| 0x021 |  | Channel 1 | monitoring | read | V(rms) | 14.091064 |  | `electrical.switches.1Power.voltage.channel1` |
| 0x022 |  | Channel 2 | monitoring | read | V(rms) | 14.073339 |  | `electrical.switches.1Power.voltage.channel2` |
| 0x023 |  | Channel 3 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.1Power.voltage.channel3` |
| 0x024 |  | Channel 4 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.1Power.voltage.channel4` |
| 0x025 |  | Channel 5 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.1Power.voltage.channel5` |
| 0x026 |  | Channel 6 | monitoring | read | V(rms) | 14.286035 |  | `electrical.switches.1Power.voltage.channel6` |
| 0x027 |  | Channel 7 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.1Power.voltage.channel7` |
| 0x028 |  | Channel 8 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.1Power.voltage.channel8` |
| 0x029 |  | Channel 9 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.1Power.voltage.channel9` |
| 0x02A |  | Channel 10 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.1Power.voltage.channel10` |
| 0x031 | Outputs | Disabled | monitoring | command / event output |  | – |  | – |
| 0x100 | Device | Language | configuration | read/write |  | – | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | Device | Name | configuration | read/write |  | 1 Power |  | – |
| 0x176 | Device | Events locked | configuration | read |  | 0 |  | – |
| 0x177 | Device | Logics locked | configuration | read |  | 0 |  | – |
| 0x1B0 | Device | Low bat alarm | configuration | read/write |  | 12V | 12V, 24V, Disabled | – |
| 0x16B | System | MasterBus power | configuration | read/write |  | On | Off, On | – |
| 0x16C | Channel 1 | Config. locked | configuration | read |  | 0 |  | – |
| 0x12D | Channel 1 | Mode | configuration | read/write |  | Motor | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled | – |
| 0x105 | Channel 1 | Name | configuration | read/write |  | Bilge Pump Front |  | – |
| 0x123 | Channel 1 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x10F | Channel 1 | Startup state | configuration | read/write |  | On | Off, On, Last | – |
| 0x1A6 | Channel 1 | Soft start | configuration | read/write |  | 0 |  | – |
| 0x155 | Channel 1 | Minimum current | configuration | read/write |  | Not set | Not set, 50 mA, 100 mA, 200 mA, 300 mA, 500 mA, 700 mA, 1 A, 2 A, 3 A, 5 A, 7 A | – |
| 0x16D | Channel 2 | Config. locked | configuration | read |  | 0 |  | – |
| 0x12E | Channel 2 | Mode | configuration | read/write |  | Motor | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x106 | Channel 2 | Name | configuration | read/write |  | Bilge Pump Rear |  | – |
| 0x124 | Channel 2 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x110 | Channel 2 | Startup state | configuration | read/write |  | On | Off, On, Last | – |
| 0x1A7 | Channel 2 | Soft start | configuration | read/write |  | 0 |  | – |
| 0x156 | Channel 2 | Minimum current | configuration | read/write |  | Not set | Not set, 50 mA, 100 mA, 200 mA, 300 mA, 500 mA, 700 mA, 1 A, 2 A, 3 A, 5 A, 7 A | – |
| 0x16E | Channel 3 | Config. locked | configuration | read |  | 0 |  | – |
| 0x12F | Channel 3 | Mode | configuration | read/write |  | Motor | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x107 | Channel 3 | Name | configuration | read/write |  | Shower Pump |  | – |
| 0x125 | Channel 3 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x111 | Channel 3 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x1A8 | Channel 3 | Soft start | configuration | read/write |  | 0 |  | – |
| 0x157 | Channel 3 | Minimum current | configuration | read/write |  | Not set | Not set, 50 mA, 100 mA, 200 mA, 300 mA, 500 mA, 700 mA, 1 A, 2 A, 3 A, 5 A, 7 A | – |
| 0x16F | Channel 4 | Config. locked | configuration | read |  | 0 |  | – |
| 0x130 | Channel 4 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x108 | Channel 4 | Name | configuration | read/write |  | Spare |  | – |
| 0x126 | Channel 4 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x112 | Channel 4 | Startup state | configuration | read/write |  | On | Off, On, Last | – |
| 0x14E | Channel 4 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x170 | Channel 5 | Config. locked | configuration | read |  | 0 |  | – |
| 0x131 | Channel 5 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x109 | Channel 5 | Name | configuration | read/write |  | Switched Power |  | – |
| 0x127 | Channel 5 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x113 | Channel 5 | Startup state | configuration | read/write |  | On | Off, On, Last | – |
| 0x14F | Channel 5 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x171 | Channel 6 | Config. locked | configuration | read |  | 0 |  | – |
| 0x132 | Channel 6 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10A | Channel 6 | Name | configuration | read/write |  | Constant Power |  | – |
| 0x128 | Channel 6 | Software fuse | configuration | read/write |  | Hardware only | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x114 | Channel 6 | Startup state | configuration | read/write |  | On | Off, On, Last | – |
| 0x150 | Channel 6 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x172 | Channel 7 | Config. locked | configuration | read |  | 0 |  | – |
| 0x133 | Channel 7 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10B | Channel 7 | Name | configuration | read/write |  | Main H5 Pilot |  | – |
| 0x129 | Channel 7 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x115 | Channel 7 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x151 | Channel 7 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x173 | Channel 8 | Config. locked | configuration | read |  | 0 |  | – |
| 0x134 | Channel 8 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10C | Channel 8 | Name | configuration | read/write |  | Backup H5 Pilot |  | – |
| 0x12A | Channel 8 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x116 | Channel 8 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x152 | Channel 8 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x174 | Channel 9 | Config. locked | configuration | read |  | 0 |  | – |
| 0x135 | Channel 9 | Mode | configuration | read/write |  | Motor | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10D | Channel 9 | Name | configuration | read/write |  | Freshwater pump |  | – |
| 0x12B | Channel 9 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x117 | Channel 9 | Startup state | configuration | read/write |  | On | Off, On, Last | – |
| 0x1AE | Channel 9 | Soft start | configuration | read/write |  | 0 |  | – |
| 0x15D | Channel 9 | Minimum current | configuration | read/write |  | Not set | Not set, 50 mA, 100 mA, 200 mA, 300 mA, 500 mA, 700 mA, 1 A, 2 A, 3 A, 5 A, 7 A | – |
| 0x175 | Channel 10 | Config. locked | configuration | read |  | 0 |  | – |
| 0x136 | Channel 10 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10E | Channel 10 | Name | configuration | read/write |  | Spare |  | – |
| 0x12C | Channel 10 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x118 | Channel 10 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x154 | Channel 10 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x1B3 | Events | Event 1 source | configuration | read/write |  | Logical block 1 | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x1B4 | Events | Event 1 target | configuration | read/write |  | 10 |  | – |
| 0x1B5 | Events | Event 1 command | configuration | read/write |  | 4 |  | – |
| 0x1B6 | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x1B7 | Events | Event 2 source | configuration | read/write |  | Shower Pump | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x1B8 | Events | Event 2 target | configuration | read/write |  | 2 |  | – |
| 0x1B9 | Events | Event 2 command | configuration | read/write |  | 1 |  | – |
| 0x1BA | Events | Event 2 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x1BB | Events | Event 3 source | configuration | read/write |  | Shower Pump | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x1BC | Events | Event 3 target | configuration | read/write |  | 2 |  | – |
| 0x1BD | Events | Event 3 command | configuration | read/write |  | 17 |  | – |
| 0x1BE | Events | Event 3 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x1BF | Events | Event 4 source | configuration | read/write |  | Disabled | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x178 | Logical block 1 | Function | configuration | read/write |  | AND | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17C | Logical block 1 | Source A | configuration | read/write |  | Bilge Pump Front | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x180 | Logical block 1 | Source B | configuration | read/write |  | Bilge Pump Rear | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x184 | Logical block 1 | Source C | configuration | read/write |  | Constant Power | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x179 | Logical block 2 | Function | configuration | read/write |  | Disabled | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17D | Logical block 2 | Source A | configuration | read/write |  | True ('1') | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x181 | Logical block 2 | Source B | configuration | read/write |  | Constant Power | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x185 | Logical block 2 | Source C | configuration | read/write |  | Logical block 1 | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x17A | Logical block 3 | Function | configuration | read/write |  | Disabled | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17E | Logical block 3 | Source A | configuration | read/write |  | False ('0') | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x182 | Logical block 3 | Source B | configuration | read/write |  | False ('0') | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x186 | Logical block 3 | Source C | configuration | read/write |  | False ('0') | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x17B | Logical block 4 | Function | configuration | read/write |  | Disabled | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17F | Logical block 4 | Source A | configuration | read/write |  | False ('0') | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x183 | Logical block 4 | Source B | configuration | read/write |  | False ('0') | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x187 | Logical block 4 | Source C | configuration | read/write |  | False ('0') | False ('0'), True ('1'), Bilge Pump Front, Bilge Pump Rear, Shower Pump, Spare, Switched Power, Constant Power, Main H5 Pilot, Backup H5 Pilot, Freshwater pump, Spare, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |

## DSI 1 Input4x

Article 77030900, firmware 2.4, serial KN27M8166, Signal K instance `1Input4x`, menus: monitoring, configuration, alarm, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | General | State | monitoring | read |  | On | On, Disabled | `electrical.switches.1Input4x.deviceState` |
| 0x003 | General | INTERNET on/off | monitoring | read |  | – |  | – |
| 0x006 | General | ONBOARD mode | monitoring | read |  | – |  | – |
| 0x009 | General | COMPUTER on/off | monitoring | read |  | – |  | – |
| 0x00C | General | NAVIGATION mode | monitoring | read |  | – |  | – |
| 0x001 | General | Disabled | monitoring | command / event output |  | – |  | – |
| 0x100 | General | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | General | Device name | configuration | read/write |  | 1 Input4x |  | – |
| 0x110 | General | Events locked | configuration | read |  | 0 |  | – |
| 0x104 | General | Switch 1 | configuration | read/write |  | INTERNET on/off |  | – |
| 0x105 | General | Switch 2 | configuration | read/write |  | ONBOARD mode |  | – |
| 0x106 | General | Switch 3 | configuration | read/write |  | COMPUTER on/off |  | – |
| 0x107 | General | Switch 4 | configuration | read/write |  | NAVIGATION mode |  | – |
| 0x112 | Events | Event 1 source | configuration | read |  | ONBOARD mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x113 | Events | Event 1 target | configuration | read |  | 6 |  | – |
| 0x114 | Events | Event 1 command | configuration | read |  | 4 |  | – |
| 0x115 | Events | Event 1 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x116 | Events | Event 2 source | configuration | read |  | ONBOARD mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x117 | Events | Event 2 target | configuration | read |  | 6 |  | – |
| 0x118 | Events | Event 2 command | configuration | read |  | 8 |  | – |
| 0x119 | Events | Event 2 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x11A | Events | Event 3 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x11B | Events | Event 3 target | configuration | read |  | 7 |  | – |
| 0x11C | Events | Event 3 command | configuration | read |  | 0 |  | – |
| 0x11D | Events | Event 3 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x11E | Events | Event 4 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x11F | Events | Event 4 target | configuration | read |  | 7 |  | – |
| 0x120 | Events | Event 4 command | configuration | read |  | 1 |  | – |
| 0x121 | Events | Event 4 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x122 | Events | Event 5 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x123 | Events | Event 5 target | configuration | read |  | 7 |  | – |
| 0x124 | Events | Event 5 command | configuration | read |  | 2 |  | – |
| 0x125 | Events | Event 5 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x126 | Events | Event 6 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x127 | Events | Event 6 target | configuration | read |  | 7 |  | – |
| 0x128 | Events | Event 6 command | configuration | read |  | 3 |  | – |
| 0x129 | Events | Event 6 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12A | Events | Event 7 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x12B | Events | Event 7 target | configuration | read |  | 4 |  | – |
| 0x12C | Events | Event 7 command | configuration | read |  | 0 |  | – |
| 0x12D | Events | Event 7 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12E | Events | Event 8 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x12F | Events | Event 8 target | configuration | read |  | 7 |  | – |
| 0x130 | Events | Event 8 command | configuration | read |  | 5 |  | – |
| 0x131 | Events | Event 8 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x132 | Events | Event 9 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x133 | Events | Event 9 target | configuration | read |  | 7 |  | – |
| 0x134 | Events | Event 9 command | configuration | read |  | 9 |  | – |
| 0x135 | Events | Event 9 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x136 | Events | Event 10 source | configuration | read |  | NAVIGATION mode | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x137 | Events | Event 10 target | configuration | read |  | 6 |  | – |
| 0x138 | Events | Event 10 command | configuration | read |  | 6 |  | – |
| 0x139 | Events | Event 10 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x13A | Events | Event 11 source | configuration | read |  | INTERNET on/off | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x13B | Events | Event 11 target | configuration | read |  | 7 |  | – |
| 0x13C | Events | Event 11 command | configuration | read |  | 7 |  | – |
| 0x13D | Events | Event 11 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x13E | Events | Event 12 source | configuration | read |  | COMPUTER on/off | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |
| 0x13F | Events | Event 12 target | configuration | read |  | 3 |  | – |
| 0x140 | Events | Event 12 command | configuration | read |  | 0 |  | – |
| 0x141 | Events | Event 12 data | configuration | read |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x142 | Events | Event 13 source | configuration | read |  | Disabled | Disabled, INTERNET on/off, ONBOARD mode, COMPUTER on/off, NAVIGATION mode | – |

## DCD 4 START ALL

Article 77020200, firmware 2.56, serial R828G5184, Signal K instance `4StartAll`, menus: monitoring, alarm, configuration, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | OK | Alarm, OK | `electrical.distribution.4StartAll.state` |
| 0x200 | Fuses | EmergencySwitch | alarm | read |  | 0 |  | `electrical.distribution.4StartAll.fuses.emergencySwitch` |
| 0x201 | Fuses | +out B | alarm | read |  | 0 |  | `electrical.distribution.4StartAll.fuses.outB` |
| 0x202 | Fuses | Unused | alarm | read |  | 0 |  | `electrical.distribution.4StartAll.fuses.unused` |
| 0x203 | Fuses | START Switch | alarm | read |  | 0 |  | `electrical.distribution.4StartAll.fuses.startSwitch` |
| 0x204 | Fuses | Temperature high | alarm | read |  | 0 |  | `electrical.distribution.4StartAll.fuses.temperatureHigh` |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | Device | Device name | configuration | read/write |  | 4 START ALL |  | – |
| 0x109 | Fuses | Fuse 1 enabled | configuration | read/write |  | 0 |  | – |
| 0x105 | Fuses | Name Fuse 1 | configuration | read/write |  | EmergencySwitch |  | – |
| 0x10A | Fuses | Fuse 2 enabled | configuration | read/write |  | 0 |  | – |
| 0x106 | Fuses | Name Fuse 2 | configuration | read/write |  | +out B |  | – |
| 0x10B | Fuses | Fuse 3 enabled | configuration | read/write |  | 0 |  | – |
| 0x107 | Fuses | Name Fuse 3 | configuration | read/write |  | Unused |  | – |
| 0x10C | Fuses | Fuse 4 enabled | configuration | read/write |  | 0 |  | – |
| 0x108 | Fuses | Name Fuse 4 | configuration | read/write |  | START Switch |  | – |
| 0x122 | Events | Event 1 source | configuration | read/write |  | Fuse 1 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x123 | Events | Event 1 target | configuration | read/write |  | 10 |  | – |
| 0x124 | Events | Event 1 command | configuration | read/write |  | 0 |  | – |
| 0x125 | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x126 | Events | Event 2 source | configuration | read/write |  | Fuse 2 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x127 | Events | Event 2 target | configuration | read/write |  | 10 |  | – |
| 0x128 | Events | Event 2 command | configuration | read/write |  | 0 |  | – |
| 0x129 | Events | Event 2 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12A | Events | Event 3 source | configuration | read/write |  | Fuse 3 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12B | Events | Event 3 target | configuration | read/write |  | 10 |  | – |
| 0x12C | Events | Event 3 command | configuration | read/write |  | 0 |  | – |
| 0x12D | Events | Event 3 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12E | Events | Event 4 source | configuration | read/write |  | Fuse 4 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12F | Events | Event 4 target | configuration | read/write |  | 10 |  | – |
| 0x130 | Events | Event 4 command | configuration | read/write |  | 0 |  | – |
| 0x131 | Events | Event 4 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x132 | Events | Event 5 source | configuration | read/write |  | Disabled | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x11C | Processor | Version | configuration | read |  | 2.56.0 |  | – |

## DCD 2 SERVICE CH

Article 77020200, firmware 2.56, serial R828G4457, Signal K instance `2ServiceCh`, menus: monitoring, alarm, configuration, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | OK | Alarm, OK | `electrical.distribution.2ServiceCh.state` |
| 0x200 | Fuses | Unused | alarm | read |  | 0 |  | `electrical.distribution.2ServiceCh.fuses.unused` |
| 0x201 | Fuses | SolarDeck Charg | alarm | read |  | 0 |  | `electrical.distribution.2ServiceCh.fuses.solarDeckCharg` |
| 0x202 | Fuses | Service Charger | alarm | read |  | 0 |  | `electrical.distribution.2ServiceCh.fuses.serviceCharger` |
| 0x203 | Fuses | MCU Chg/Inv/Sol | alarm | read |  | 0 |  | `electrical.distribution.2ServiceCh.fuses.mcuChgInvSol` |
| 0x204 | Fuses | Temperature high | alarm | read |  | 0 |  | `electrical.distribution.2ServiceCh.fuses.temperatureHigh` |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | Device | Device name | configuration | read/write |  | 2 SERVICE CH |  | – |
| 0x109 | Fuses | Fuse 1 enabled | configuration | read/write |  | 0 |  | – |
| 0x105 | Fuses | Name Fuse 1 | configuration | read/write |  | Unused |  | – |
| 0x10A | Fuses | Fuse 2 enabled | configuration | read/write |  | 0 |  | – |
| 0x106 | Fuses | Name Fuse 2 | configuration | read/write |  | SolarDeck Charg |  | – |
| 0x10B | Fuses | Fuse 3 enabled | configuration | read/write |  | 0 |  | – |
| 0x107 | Fuses | Name Fuse 3 | configuration | read/write |  | Service Charger |  | – |
| 0x10C | Fuses | Fuse 4 enabled | configuration | read/write |  | 0 |  | – |
| 0x108 | Fuses | Name Fuse 4 | configuration | read/write |  | MCU Chg/Inv/Sol |  | – |
| 0x122 | Events | Event 1 source | configuration | read/write |  | Fuse 1 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x123 | Events | Event 1 target | configuration | read/write |  | 10 |  | – |
| 0x124 | Events | Event 1 command | configuration | read/write |  | 0 |  | – |
| 0x125 | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x126 | Events | Event 2 source | configuration | read/write |  | Fuse 2 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x127 | Events | Event 2 target | configuration | read/write |  | 10 |  | – |
| 0x128 | Events | Event 2 command | configuration | read/write |  | 0 |  | – |
| 0x129 | Events | Event 2 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12A | Events | Event 3 source | configuration | read/write |  | Fuse 3 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12B | Events | Event 3 target | configuration | read/write |  | 10 |  | – |
| 0x12C | Events | Event 3 command | configuration | read/write |  | 0 |  | – |
| 0x12D | Events | Event 3 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12E | Events | Event 4 source | configuration | read/write |  | Fuse 4 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12F | Events | Event 4 target | configuration | read/write |  | 10 |  | – |
| 0x130 | Events | Event 4 command | configuration | read/write |  | 0 |  | – |
| 0x131 | Events | Event 4 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x132 | Events | Event 5 source | configuration | read/write |  | Disabled | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x11C | Processor | Version | configuration | read |  | 2.56.0 |  | – |

## DCD 3 VITAL ALL

Article 77020200, firmware 2.56, serial JO23G7815, Signal K instance `3VitalAll`, menus: monitoring, alarm, configuration, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | OK | Alarm, OK | `electrical.distribution.3VitalAll.state` |
| 0x200 | Fuses | VITAL Charger | alarm | read |  | 0 |  | `electrical.distribution.3VitalAll.fuses.vitalCharger` |
| 0x201 | Fuses | VITAL Switch | alarm | read |  | 0 |  | `electrical.distribution.3VitalAll.fuses.vitalSwitch` |
| 0x202 | Fuses | DDC-1 Power | alarm | read |  | 0 |  | `electrical.distribution.3VitalAll.fuses.ddc1Power` |
| 0x203 | Fuses | VITAL Constant | alarm | read |  | 0 |  | `electrical.distribution.3VitalAll.fuses.vitalConstant` |
| 0x204 | Fuses | Temperature high | alarm | read |  | 0 |  | `electrical.distribution.3VitalAll.fuses.temperatureHigh` |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | Device | Device name | configuration | read/write |  | 3 VITAL ALL |  | – |
| 0x109 | Fuses | Fuse 1 enabled | configuration | read/write |  | 0 |  | – |
| 0x105 | Fuses | Name Fuse 1 | configuration | read/write |  | VITAL Charger |  | – |
| 0x10A | Fuses | Fuse 2 enabled | configuration | read/write |  | 0 |  | – |
| 0x106 | Fuses | Name Fuse 2 | configuration | read/write |  | VITAL Switch |  | – |
| 0x10B | Fuses | Fuse 3 enabled | configuration | read/write |  | 0 |  | – |
| 0x107 | Fuses | Name Fuse 3 | configuration | read/write |  | DDC-1 Power |  | – |
| 0x10C | Fuses | Fuse 4 enabled | configuration | read/write |  | 0 |  | – |
| 0x108 | Fuses | Name Fuse 4 | configuration | read/write |  | VITAL Constant |  | – |
| 0x122 | Events | Event 1 source | configuration | read/write |  | Fuse 1 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x123 | Events | Event 1 target | configuration | read/write |  | 10 |  | – |
| 0x124 | Events | Event 1 command | configuration | read/write |  | 0 |  | – |
| 0x125 | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x126 | Events | Event 2 source | configuration | read/write |  | Fuse 2 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x127 | Events | Event 2 target | configuration | read/write |  | 10 |  | – |
| 0x128 | Events | Event 2 command | configuration | read/write |  | 0 |  | – |
| 0x129 | Events | Event 2 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12A | Events | Event 3 source | configuration | read/write |  | Fuse 3 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12B | Events | Event 3 target | configuration | read/write |  | 10 |  | – |
| 0x12C | Events | Event 3 command | configuration | read/write |  | 0 |  | – |
| 0x12D | Events | Event 3 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12E | Events | Event 4 source | configuration | read/write |  | Fuse 4 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12F | Events | Event 4 target | configuration | read/write |  | 10 |  | – |
| 0x130 | Events | Event 4 command | configuration | read/write |  | 0 |  | – |
| 0x131 | Events | Event 4 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x132 | Events | Event 5 source | configuration | read/write |  | Disabled | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x11C | Processor | Version | configuration | read |  | 2.56.0 |  | – |

## DCD 1 SERVICE LD

Article 77020200, firmware 2.56, serial JO23G9975, Signal K instance `1ServiceLd`, menus: monitoring, alarm, configuration, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Device | State | monitoring | read |  | OK | Alarm, OK | `electrical.distribution.1ServiceLd.state` |
| 0x200 | Fuses | SERVICE switch | alarm | read |  | 0 |  | `electrical.distribution.1ServiceLd.fuses.serviceSwitch` |
| 0x201 | Fuses | Bow Charger | alarm | read |  | 0 |  | `electrical.distribution.1ServiceLd.fuses.bowCharger` |
| 0x202 | Fuses | VITAL Charger | alarm | read |  | 0 |  | `electrical.distribution.1ServiceLd.fuses.vitalCharger` |
| 0x203 | Fuses | Unused | alarm | read |  | 0 |  | `electrical.distribution.1ServiceLd.fuses.unused` |
| 0x204 | Fuses | Temperature high | alarm | read |  | 0 |  | `electrical.distribution.1ServiceLd.fuses.temperatureHigh` |
| 0x100 | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | Device | Device name | configuration | read/write |  | 1 SERVICE LD |  | – |
| 0x109 | Fuses | Fuse 1 enabled | configuration | read/write |  | 0 |  | – |
| 0x105 | Fuses | Name Fuse 1 | configuration | read/write |  | SERVICE switch |  | – |
| 0x10A | Fuses | Fuse 2 enabled | configuration | read/write |  | 0 |  | – |
| 0x106 | Fuses | Name Fuse 2 | configuration | read/write |  | Bow Charger |  | – |
| 0x10B | Fuses | Fuse 3 enabled | configuration | read/write |  | 0 |  | – |
| 0x107 | Fuses | Name Fuse 3 | configuration | read/write |  | VITAL Charger |  | – |
| 0x10C | Fuses | Fuse 4 enabled | configuration | read/write |  | 0 |  | – |
| 0x108 | Fuses | Name Fuse 4 | configuration | read/write |  | Unused |  | – |
| 0x122 | Events | Event 1 source | configuration | read/write |  | Fuse 1 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x123 | Events | Event 1 target | configuration | read/write |  | 10 |  | – |
| 0x124 | Events | Event 1 command | configuration | read/write |  | 0 |  | – |
| 0x125 | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x126 | Events | Event 2 source | configuration | read/write |  | Fuse 2 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x127 | Events | Event 2 target | configuration | read/write |  | 10 |  | – |
| 0x128 | Events | Event 2 command | configuration | read/write |  | 0 |  | – |
| 0x129 | Events | Event 2 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12A | Events | Event 3 source | configuration | read/write |  | Fuse 3 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12B | Events | Event 3 target | configuration | read/write |  | 10 |  | – |
| 0x12C | Events | Event 3 command | configuration | read/write |  | 0 |  | – |
| 0x12D | Events | Event 3 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x12E | Events | Event 4 source | configuration | read/write |  | Fuse 4 fail | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x12F | Events | Event 4 target | configuration | read/write |  | 10 |  | – |
| 0x130 | Events | Event 4 command | configuration | read/write |  | 0 |  | – |
| 0x131 | Events | Event 4 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x132 | Events | Event 5 source | configuration | read/write |  | Disabled | Disabled, State OK, Fuse 1 fail, Fuse 2 fail, Fuse 3 fail, Fuse 4 fail, Temperature high | – |
| 0x11C | Processor | Version | configuration | read |  | 2.56.0 |  | – |

## DSD 2 Switch

Article 77020400, firmware 2.22, serial J308E4371, Signal K instance `2Switch`, menus: monitoring, alarm, history, configuration.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x030 | General | Device state | monitoring | read |  | OK | Disabled, OK, Alarm | `electrical.switches.2Switch.deviceState` |
| 0x003 | Channels | N2K net 1 | monitoring | event output |  | True |  | `electrical.switches.2Switch.n2kNet1.state` (put) |
| 0x005 | Channels | N2K net 2 | monitoring | event output |  | False |  | `electrical.switches.2Switch.n2kNet2.state` (put) |
| 0x007 | Channels | N2K net 3 | monitoring | event output |  | True |  | `electrical.switches.2Switch.n2kNet3.state` (put) |
| 0x009 | Channels | N2K net 4 | monitoring | event output |  | True |  | `electrical.switches.2Switch.n2kNet4.state` (put) |
| 0x00B | Channels | Linux Pi3 | monitoring | event output |  | True |  | `electrical.switches.2Switch.linuxPi3.state` |
| 0x00D | Channels | Nemesis | monitoring | event output |  | False |  | `electrical.switches.2Switch.nemesis.state` (put) |
| 0x00F | Channels | LAN Router | monitoring | event output |  | True |  | `electrical.switches.2Switch.lanRouter.state` |
| 0x011 | Channels | Starlink | monitoring | event output |  | True |  | `electrical.switches.2Switch.starlink.state` (put) |
| 0x015 | Channels | Radar+ SonarHub | monitoring | event output |  | False |  | `electrical.switches.2Switch.radarSonarHub.state` (put) |
| 0x02E | Reset | Reset alarms | monitoring | event output |  | False |  | `electrical.switches.2Switch.reset.resetAlarms` (put) |
| 0x21E | Alarm status | N2K net 1 | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.n2kNet1` |
| 0x21F | Alarm status | N2K net 2 | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.n2kNet2` |
| 0x220 | Alarm status | N2K net 3 | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.n2kNet3` |
| 0x221 | Alarm status | N2K net 4 | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.n2kNet4` |
| 0x222 | Alarm status | Linux Pi3 | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.linuxPi3` |
| 0x223 | Alarm status | Nemesis | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.nemesis` |
| 0x224 | Alarm status | LAN Router | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.lanRouter` |
| 0x225 | Alarm status | Starlink | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.starlink` |
| 0x227 | Alarm status | Radar+ SonarHub | alarm | read |  | No alarms | No alarms, Minimum current, Hardware fuse, Overload | `electrical.switches.2Switch.alarmStatus.radarSonarHub` |
| 0x22A | System | Temperature | alarm | read |  | Normal | Normal, High, Critical | `electrical.switches.2Switch.system.temperature` |
| 0x22B | System | Battery Low | alarm | read |  | 0 |  | `electrical.switches.2Switch.system.batteryLow` |
| 0x017 |  | Channel 1 | monitoring | read | A(rms) | 0.83128864 |  | `electrical.switches.2Switch.current.channel1` |
| 0x018 |  | Channel 2 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.2Switch.current.channel2` |
| 0x019 |  | Channel 3 | monitoring | read | A(rms) | 0.04184977 |  | `electrical.switches.2Switch.current.channel3` |
| 0x01A |  | Channel 4 | monitoring | read | A(rms) | 0.25680542 |  | `electrical.switches.2Switch.current.channel4` |
| 0x01B |  | Channel 5 | monitoring | read | A(rms) | 0.4242045 |  | `electrical.switches.2Switch.current.channel5` |
| 0x01C |  | Channel 6 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.2Switch.current.channel6` |
| 0x01D |  | Channel 7 | monitoring | read | A(rms) | 0.26441446 |  | `electrical.switches.2Switch.current.channel7` |
| 0x01E |  | Channel 8 | monitoring | read | A(rms) | 1.7291565 |  | `electrical.switches.2Switch.current.channel8` |
| 0x01F |  | Channel 9 | monitoring | read | A(rms) | 1.3620198 |  | `electrical.switches.2Switch.current.channel9` |
| 0x020 |  | Channel 10 | monitoring | read | A(rms) | 0.0 |  | `electrical.switches.2Switch.current.channel10` |
| 0x021 |  | Channel 1 | monitoring | read | V(rms) | 14.03789 |  | `electrical.switches.2Switch.voltage.channel1` |
| 0x022 |  | Channel 2 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.2Switch.voltage.channel2` |
| 0x023 |  | Channel 3 | monitoring | read | V(rms) | 14.197412 |  | `electrical.switches.2Switch.voltage.channel3` |
| 0x024 |  | Channel 4 | monitoring | read | V(rms) | 14.1265135 |  | `electrical.switches.2Switch.voltage.channel4` |
| 0x025 |  | Channel 5 | monitoring | read | V(rms) | 14.1531 |  | `electrical.switches.2Switch.voltage.channel5` |
| 0x026 |  | Channel 6 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.2Switch.voltage.channel6` |
| 0x027 |  | Channel 7 | monitoring | read | V(rms) | 14.099926 |  | `electrical.switches.2Switch.voltage.channel7` |
| 0x028 |  | Channel 8 | monitoring | read | V(rms) | 14.1087885 |  | `electrical.switches.2Switch.voltage.channel8` |
| 0x029 |  | Channel 9 | monitoring | read | V(rms) | 14.1265135 |  | `electrical.switches.2Switch.voltage.channel9` |
| 0x02A |  | Channel 10 | monitoring | read | V(rms) | 0.0 |  | `electrical.switches.2Switch.voltage.channel10` |
| 0x013 | Outputs | Linked | monitoring | event output |  | – |  | – |
| 0x031 | Outputs | Disabled | monitoring | command / event output |  | – |  | – |
| 0x100 | Device | Language | configuration | read/write |  | – | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x103 | Device | Name | configuration | read/write |  | 2 Switch |  | – |
| 0x176 | Device | Events locked | configuration | read |  | 0 |  | – |
| 0x177 | Device | Logics locked | configuration | read |  | 0 |  | – |
| 0x1B0 | Device | Low bat alarm | configuration | read/write |  | 12V | 12V, 24V, Disabled | – |
| 0x16B | System | MasterBus power | configuration | read/write |  | On | Off, On | – |
| 0x16C | Channel 1 | Config. locked | configuration | read |  | 0 |  | – |
| 0x12D | Channel 1 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled | – |
| 0x105 | Channel 1 | Name | configuration | read/write |  | N2K net 1 |  | – |
| 0x123 | Channel 1 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x10F | Channel 1 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x14B | Channel 1 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x16D | Channel 2 | Config. locked | configuration | read |  | 0 |  | – |
| 0x12E | Channel 2 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x106 | Channel 2 | Name | configuration | read/write |  | N2K net 2 |  | – |
| 0x124 | Channel 2 | Software fuse | configuration | read/write |  | 5 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x110 | Channel 2 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x14C | Channel 2 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x16E | Channel 3 | Config. locked | configuration | read |  | 0 |  | – |
| 0x12F | Channel 3 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x107 | Channel 3 | Name | configuration | read/write |  | N2K net 3 |  | – |
| 0x125 | Channel 3 | Software fuse | configuration | read/write |  | 5 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x111 | Channel 3 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x14D | Channel 3 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x16F | Channel 4 | Config. locked | configuration | read |  | 0 |  | – |
| 0x130 | Channel 4 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x108 | Channel 4 | Name | configuration | read/write |  | N2K net 4 |  | – |
| 0x126 | Channel 4 | Software fuse | configuration | read/write |  | 5 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x112 | Channel 4 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x14E | Channel 4 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x170 | Channel 5 | Config. locked | configuration | read |  | 0 |  | – |
| 0x131 | Channel 5 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x109 | Channel 5 | Name | configuration | read/write |  | Linux Pi3 |  | – |
| 0x127 | Channel 5 | Software fuse | configuration | read/write |  | 4 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x113 | Channel 5 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x14F | Channel 5 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x171 | Channel 6 | Config. locked | configuration | read |  | 0 |  | – |
| 0x132 | Channel 6 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10A | Channel 6 | Name | configuration | read/write |  | Nemesis |  | – |
| 0x128 | Channel 6 | Software fuse | configuration | read/write |  | 5 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x114 | Channel 6 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x150 | Channel 6 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x172 | Channel 7 | Config. locked | configuration | read |  | 0 |  | – |
| 0x133 | Channel 7 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10B | Channel 7 | Name | configuration | read/write |  | LAN Router |  | – |
| 0x129 | Channel 7 | Software fuse | configuration | read/write |  | 3 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x115 | Channel 7 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x151 | Channel 7 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x173 | Channel 8 | Config. locked | configuration | read |  | 0 |  | – |
| 0x134 | Channel 8 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10C | Channel 8 | Name | configuration | read/write |  | Starlink |  | – |
| 0x12A | Channel 8 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x116 | Channel 8 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x152 | Channel 8 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x174 | Channel 9 | Config. locked | configuration | read |  | 0 |  | – |
| 0x135 | Channel 9 | Mode | configuration | read/write |  | Linked | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x12B | Channel 9 | Software fuse | configuration | read/write |  | 10 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x15D | Channel 9 | Minimum current | configuration | read/write |  | Not set | Not set, 50 mA, 100 mA, 200 mA, 300 mA, 500 mA, 700 mA, 1 A, 2 A, 3 A, 5 A, 7 A | – |
| 0x175 | Channel 10 | Config. locked | configuration | read |  | 0 |  | – |
| 0x136 | Channel 10 | Mode | configuration | read/write |  | Conventional | Conventional, Light 12V, Light 24V, Switch light 12V, Switch light 24V, Motor, Bilge pump float, Input, Manual, Always on, Disabled, Linked | – |
| 0x10E | Channel 10 | Name | configuration | read/write |  | Radar+ SonarHub |  | – |
| 0x12C | Channel 10 | Software fuse | configuration | read/write |  | 7.5 A | 2 A, 3 A, 4 A, 5 A, 7.5 A, 10 A, Hardware only | – |
| 0x118 | Channel 10 | Startup state | configuration | read/write |  | Off | Off, On, Last | – |
| 0x154 | Channel 10 | Delayed off | configuration | read/write |  | Immediate | Immediate, 1 second, 2 seconds, 3 seconds, 5 seconds, 10 seconds, 20 seconds, 30 seconds, 1 minute, 2 minutes, 3 minutes, 5 minutes, 10 minutes, 20 minutes, 30 minutes, 1 hour, 2 hours, 3 hours, 5 hours, 12 hours | – |
| 0x1B3 | Events | Event 1 source | configuration | read/write |  | Linux Pi3 | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x1B4 | Events | Event 1 target | configuration | read/write |  | 10 |  | – |
| 0x1B5 | Events | Event 1 command | configuration | read/write |  | 7 |  | – |
| 0x1B6 | Events | Event 1 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x1B7 | Events | Event 2 source | configuration | read/write |  | LAN Router | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x1B8 | Events | Event 2 target | configuration | read/write |  | 10 |  | – |
| 0x1B9 | Events | Event 2 command | configuration | read/write |  | 6 |  | – |
| 0x1BA | Events | Event 2 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x1BB | Events | Event 3 source | configuration | read/write |  | Logical block 1 | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x1BC | Events | Event 3 target | configuration | read/write |  | 10 |  | – |
| 0x1BD | Events | Event 3 command | configuration | read/write |  | 5 |  | – |
| 0x1BE | Events | Event 3 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x1BF | Events | Event 4 source | configuration | read/write |  | Logical block 2 | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x1C0 | Events | Event 4 target | configuration | read/write |  | 8 |  | – |
| 0x1C1 | Events | Event 4 command | configuration | read/write |  | 6 |  | – |
| 0x1C2 | Events | Event 4 data | configuration | read/write |  | Copy | Off, On, Copy, Copy invert, Toggle | – |
| 0x1C3 | Events | Event 5 source | configuration | read/write |  | Disabled | Disabled, Device state, Overload alarm, Minimum current, Hardware alarm, N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4, Battery Low | – |
| 0x178 | Logical block 1 | Function | configuration | read/write |  | AND | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17C | Logical block 1 | Source A | configuration | read/write |  | N2K net 1 | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x180 | Logical block 1 | Source B | configuration | read/write |  | N2K net 3 | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x184 | Logical block 1 | Source C | configuration | read/write |  | N2K net 4 | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x179 | Logical block 2 | Function | configuration | read/write |  | OR | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17D | Logical block 2 | Source A | configuration | read/write |  | Radar+ SonarHub | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x181 | Logical block 2 | Source B | configuration | read/write |  | Starlink | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x185 | Logical block 2 | Source C | configuration | read/write |  | Do not use | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x17A | Logical block 3 | Function | configuration | read/write |  | Disabled | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17E | Logical block 3 | Source A | configuration | read/write |  | False ('0') | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x182 | Logical block 3 | Source B | configuration | read/write |  | False ('0') | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x186 | Logical block 3 | Source C | configuration | read/write |  | False ('0') | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x17B | Logical block 4 | Function | configuration | read/write |  | Disabled | Disabled, AND, NAND, OR, NOR, XOR, NXOR | – |
| 0x17F | Logical block 4 | Source A | configuration | read/write |  | False ('0') | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x183 | Logical block 4 | Source B | configuration | read/write |  | False ('0') | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |
| 0x187 | Logical block 4 | Source C | configuration | read/write |  | False ('0') | False ('0'), True ('1'), N2K net 1, N2K net 2, N2K net 3, N2K net 4, Linux Pi3, Nemesis, LAN Router, Starlink, Do not use, Radar+ SonarHub, Logical block 1, Logical block 2, Logical block 3, Logical block 4 | – |

## BAT SERVICE

Article 66016000, firmware 2.15, serial PO25B0222, Signal K instance `service`, menus: monitoring, alarm, history, configuration.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Battery | State of charge | monitoring | read | % | 100.0 |  | `electrical.batteries.service.capacity.stateOfCharge` |
| 0x004 | Battery | Time remaining | monitoring | read |  | – |  | `electrical.batteries.service.capacity.timeRemaining` |
| 0x001 | Battery | Voltage | monitoring | read | V | 13.499272 |  | `electrical.batteries.service.voltage` |
| 0x002 | Battery | Current | monitoring | read | A | 0.0 |  | `electrical.batteries.service.current` |
| 0x005 | Battery | Temperature | monitoring | read | °C | 9.0 |  | `electrical.batteries.service.temperature` |
| 0x022 | Relay | Relay close | monitoring | event output |  | – |  | – |
| 0x024 | Relay | Relay open | monitoring | event output |  | – |  | – |
| 0x026 | Relay | Relay state | monitoring | read |  | Closed | Closed, Open, Not connected, Connected, Waiting, Closing | `electrical.batteries.service.relay.relayState` |
| 0x200 | General | Battery safety | alarm | read |  | 0 |  | `electrical.batteries.service.general.batterySafety` |
| 0x201 | General | Over current | alarm | read |  | 0 |  | `electrical.batteries.service.general.overCurrent` |
| 0x202 | General | Capacity low | alarm | read |  | 0 |  | `electrical.batteries.service.general.capacityLow` |
| 0x203 | General | Cap. very low | alarm | read |  | 0 |  | `electrical.batteries.service.general.capVeryLow` |
| 0x204 | General | Check relay | alarm | read |  | 0 |  | `electrical.batteries.service.general.checkRelay` |
| 0x300 | General | Cycle count | history | read |  | 33.626232 |  | `electrical.batteries.service.general.cycleCount` |
| 0x301 | General | Last time 100% | history | read | days | 0.0 |  | `electrical.batteries.service.general.lastTime100` |
| 0x302 | General | Max. Ah used | history | read | Ah | 448.88486 |  | `electrical.batteries.service.general.maxAhUsed` |
| 0x303 | General | UBat lowest | history | read | V | 12.617001 |  | `electrical.batteries.service.general.uBatLowest.voltage` |
| 0x304 | General | UBat highest | history | read | V | 14.375001 |  | `electrical.batteries.service.general.uBatHighest.voltage` |
| 0x307 | General | SOH | history | read | % | 99.80785 |  | `electrical.batteries.service.general.soh` |
| 0x103 | General | Name | configuration | read/write |  | SERVICE |  | – |
| 0x105 | General | Language | configuration | read/write |  | English | English, Dutch | – |
| 0x102 | General | Serial number | configuration | read |  | PO25B0222 |  | – |
| 0x169 | General | CZone enabled | configuration | read |  | 0 |  | – |
| 0x16A | General | DIP switches | configuration | read |  | 11 00000000 |  | – |
| 0x16E | Events | Event 1 source | configuration | read/write |  | Stop charge | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x16F | Events | Event 1 target | configuration | read/write |  | 0 |  | – |
| 0x170 | Events | Event 1 command | configuration | read/write |  | 1 |  | – |
| 0x171 | Events | Event 1 data | configuration | read/write |  | Copy Invert | Off, On, Copy, Copy Invert, Toggle | – |
| 0x172 | Events | Event 2 source | configuration | read/write |  | Battery safety | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x173 | Events | Event 2 target | configuration | read/write |  | 0 |  | – |
| 0x174 | Events | Event 2 command | configuration | read/write |  | 1 |  | – |
| 0x175 | Events | Event 2 data | configuration | read/write |  | Off | Off, On, Copy, Copy Invert, Toggle | – |
| 0x176 | Events | Event 3 source | configuration | read/write |  | Stop charge | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x177 | Events | Event 3 target | configuration | read/write |  | 5 |  | – |
| 0x178 | Events | Event 3 command | configuration | read/write |  | 0 |  | – |
| 0x179 | Events | Event 3 data | configuration | read/write |  | Copy Invert | Off, On, Copy, Copy Invert, Toggle | – |
| 0x17A | Events | Event 4 source | configuration | read/write |  | Battery safety | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x17B | Events | Event 4 target | configuration | read/write |  | 5 |  | – |
| 0x17C | Events | Event 4 command | configuration | read/write |  | 0 |  | – |
| 0x17D | Events | Event 4 data | configuration | read/write |  | Off | Off, On, Copy, Copy Invert, Toggle | – |
| 0x17E | Events | Event 5 source | configuration | read/write |  | Battery < 40% | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x17F | Events | Event 5 target | configuration | read/write |  | 11 |  | – |
| 0x180 | Events | Event 5 command | configuration | read/write |  | 0 |  | – |
| 0x181 | Events | Event 5 data | configuration | read/write |  | Copy | Off, On, Copy, Copy Invert, Toggle | – |
| 0x182 | Events | Event 6 source | configuration | read/write |  | Battery < 60% | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x183 | Events | Event 6 target | configuration | read/write |  | 0 |  | – |
| 0x184 | Events | Event 6 command | configuration | read/write |  | 0 |  | – |
| 0x185 | Events | Event 6 data | configuration | read/write |  | Off | Off, On, Copy, Copy Invert, Toggle | – |
| 0x186 | Events | Event 7 source | configuration | read/write |  | Battery < 60% | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x187 | Events | Event 7 target | configuration | read/write |  | 3 |  | – |
| 0x188 | Events | Event 7 command | configuration | read/write |  | 0 |  | – |
| 0x189 | Events | Event 7 data | configuration | read/write |  | Off | Off, On, Copy, Copy Invert, Toggle | – |
| 0x18A | Events | Event 8 source | configuration | read/write |  | Battery < 60% | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x18B | Events | Event 8 target | configuration | read/write |  | 1 |  | – |
| 0x18C | Events | Event 8 command | configuration | read/write |  | 0 |  | – |
| 0x18D | Events | Event 8 data | configuration | read/write |  | Off | Off, On, Copy, Copy Invert, Toggle | – |
| 0x18E | Events | Event 9 source | configuration | read/write |  | Battery < 40% | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x18F | Events | Event 9 target | configuration | read/write |  | 10 |  | – |
| 0x190 | Events | Event 9 command | configuration | read/write |  | 12 |  | – |
| 0x191 | Events | Event 9 data | configuration | read/write |  | Copy | Off, On, Copy, Copy Invert, Toggle | – |
| 0x192 | Events | Event 10 source | configuration | read/write |  | Stop charge | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x193 | Events | Event 10 target | configuration | read/write |  | 10 |  | – |
| 0x194 | Events | Event 10 command | configuration | read/write |  | 11 |  | – |
| 0x195 | Events | Event 10 data | configuration | read/write |  | Copy | Off, On, Copy, Copy Invert, Toggle | – |
| 0x196 | Events | Event 11 source | configuration | read/write |  | Battery safety | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x197 | Events | Event 11 target | configuration | read/write |  | 10 |  | – |
| 0x198 | Events | Event 11 command | configuration | read/write |  | 10 |  | – |
| 0x199 | Events | Event 11 data | configuration | read/write |  | Copy | Off, On, Copy, Copy Invert, Toggle | – |
| 0x19A | Events | Event 12 source | configuration | read/write |  | Battery safety | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x19B | Events | Event 12 target | configuration | read/write |  | 12 |  | – |
| 0x19C | Events | Event 12 command | configuration | read/write |  | 0 |  | – |
| 0x19D | Events | Event 12 data | configuration | read/write |  | Off | Off, On, Copy, Copy Invert, Toggle | – |
| 0x19E | Events | Event 13 source | configuration | read/write |  | Capacity low | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x19F | Events | Event 13 target | configuration | read/write |  | 10 |  | – |
| 0x1A0 | Events | Event 13 command | configuration | read/write |  | 13 |  | – |
| 0x1A1 | Events | Event 13 data | configuration | read/write |  | Copy | Off, On, Copy, Copy Invert, Toggle | – |
| 0x1A2 | Events | Event 14 source | configuration | read/write |  | Cap. very low | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x1A3 | Events | Event 14 target | configuration | read/write |  | 10 |  | – |
| 0x1A4 | Events | Event 14 command | configuration | read/write |  | 14 |  | – |
| 0x1A5 | Events | Event 14 data | configuration | read/write |  | Copy | Off, On, Copy, Copy Invert, Toggle | – |
| 0x1A6 | Events | Event 15 source | configuration | read/write |  | Disabled | Disabled, Battery safety, Stop charge, Charging, Capacity low, Cap. very low, Battery full, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, High temperature, Low temperature, Voltage low, Last time 100%, Relay open, Relay close, Over current, Battery < 20%, Battery < 40%, Battery < 60%, Battery < 80%, Battery < 100% | – |
| 0x133 | Processor 1 | Version | configuration | read |  | 2.15.0 |  | – |
| 0x135 | Processor 1 | Revision | configuration | read |  | AB0215 |  | – |

## DIS

Article 77010310, firmware 2.14, serial K903B7371, Signal K instance `dis`, menus: monitoring, configuration, alarm, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x026 | General | Language | monitoring | read/write |  | – | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk, Russian | – |
| 0x02D | General | Alarm sound | monitoring | read/write |  | – |  | – |
| 0x020 | Switches | BOILER | monitoring | read/write |  | – |  | – |
| 0x021 | Switches | CHARGERS | monitoring | read/write |  | – |  | – |
| 0x022 | Switches | SAFETY | monitoring | read/write |  | – |  | – |
| 0x023 | Switches | MONITOR NAV | monitoring | read/write |  | – |  | – |
| 0x024 | Switches | LAN ROUTER | monitoring | read/write |  | – |  | – |
| 0x025 | Switches | SIGNALK | monitoring | read/write |  | – |  | – |
| 0x028 | Power save | Standby time | monitoring | read/write |  | – | 2 minutes, 5 minutes, 10 minutes, Always on | – |
| 0x029 | Power save | Auto off | monitoring | read/write |  | – | Always on, 1 day, 2 days | – |
| 0x02A | Power save | Backlight | monitoring | read/write | % | – |  | – |
| 0x02B | Widgets | Page duration | monitoring | read/write |  | – | 15 seconds, 20 seconds, 30 seconds, 40 seconds, 50 seconds, 60 seconds | – |
| 0x02C | Widgets | Slideshow | monitoring | read/write |  | – |  | – |
| 0x000 |  | State | monitoring | event output |  | 2 | Off, On, Alarm | `electrical.switches.easyView.mode` |
| 0x004 |  | Feed back 1 | monitoring | event output |  | 0 | Off, On, Alarm | `electrical.switches.easyView.boiler.mode` |
| 0x006 |  | Feed back 2 | monitoring | event output |  | 1 | Off, On, Alarm | `electrical.switches.easyView.chargers.mode` |
| 0x008 |  | Feed back 3 | monitoring | event output |  | 1 | Off, On, Alarm | `electrical.switches.easyView.safety.mode` |
| 0x00A |  | Feed back 4 | monitoring | event output |  | 1 | Off, On, Alarm | `electrical.switches.easyView.monitorNav.mode` |
| 0x00C |  | Feed back 5 | monitoring | event output |  | 1 | Off, On, Alarm | `electrical.switches.easyView.lanRouter.mode` |
| 0x00E |  | Feed back 6 | monitoring | event output |  | 1 | Off, On, Alarm | `electrical.switches.easyView.signalk.mode` |
| 0x002 | Outputs | Lock | monitoring | command / event output |  | – |  | – |
| 0x010 | Outputs | Buzzer | monitoring | event output |  | – |  | – |
| 0x012 | Outputs | Power save | monitoring | command / event output |  | – |  | – |
| 0x014 | Outputs | Message 1 | monitoring | command / event output |  | – |  | – |
| 0x016 | Outputs | Message 2 | monitoring | command / event output |  | – |  | – |
| 0x018 | Outputs | Message 3 | monitoring | command / event output |  | – |  | – |
| 0x01A | Outputs | Message 4 | monitoring | command / event output |  | – |  | – |
| 0x01C | Outputs | Message 5 | monitoring | command / event output |  | – |  | – |
| 0x01E | Outputs | Message 6 | monitoring | command / event output |  | – |  | – |
| 0x102 | General | System name | configuration | read/write |  | (removed: boat name) |  | – |
| 0x103 | General | Device name | configuration | read/write |  |   |  | – |
| 0x160 | General | Switch 1 | configuration | read/write |  | BOILER |  | – |
| 0x161 | General | Switch 2 | configuration | read/write |  | CHARGERS |  | – |
| 0x162 | General | Switch 3 | configuration | read/write |  | SAFETY |  | – |
| 0x163 | General | Switch 4 | configuration | read/write |  | MONITOR NAV |  | – |
| 0x164 | General | Switch 5 | configuration | read/write |  | LAN ROUTER |  | – |
| 0x165 | General | Switch 6 | configuration | read/write |  | SIGNALK |  | – |
| 0x104 | General | Devices in list | configuration | read/write |  | All | Selection, All, Excluded | – |
| 0x10A | General | Auto Lock after | configuration | read/write |  | Off | Off, 2 minutes, 5 minutes, 10 minutes | – |
| 0x166 | General | Alarm | configuration | read/write |  | All | Selection, All | – |
| 0x168 | General | Factory reset | configuration | read/write |  | 0.0 |  | – |
| 0x182 | General | Rst. startup img | configuration | read/write |  | 0.0 |  | – |
| 0x169 | General | Show edit b. | configuration | read/write |  | 0 |  | – |
| 0x16A | Messages | Message 1 | configuration | read/write |  | Message 1 |  | – |
| 0x16B | Messages | Message text 1 | configuration | read/write |  | BMS: |  | – |
| 0x16C | Messages | Message text 1 | configuration | read/write |  | Battery Safety! |  | – |
| 0x16D | Messages | Message text 1 | configuration | read/write |  | BMS Relay Off |  | – |
| 0x16E | Messages | Message 2 | configuration | read/write |  | Message 2 |  | – |
| 0x16F | Messages | Message text 2 | configuration | read/write |  | BMS: |  | – |
| 0x170 | Messages | Message text 2 | configuration | read/write |  | Stop Charging |  | – |
| 0x171 | Messages | Message text 2 | configuration | read/write |  | Active! |  | – |
| 0x172 | Messages | Message 3 | configuration | read/write |  | Message 3 |  | – |
| 0x173 | Messages | Message text 3 | configuration | read/write |  | BMS: |  | – |
| 0x174 | Messages | Message text 3 | configuration | read/write |  | SOC < 40% |  | – |
| 0x175 | Messages | Message text 3 | configuration | read/write |  | – |  | – |
| 0x176 | Messages | Message 4 | configuration | read/write |  | Message 4 |  | – |
| 0x177 | Messages | Message text 4 | configuration | read/write |  | BMS: |  | – |
| 0x178 | Messages | Message text 4 | configuration | read/write |  | <20% Capacity |  | – |
| 0x179 | Messages | Message text 4 | configuration | read/write |  | LOW Power mode! |  | – |
| 0x17A | Messages | Message 5 | configuration | read/write |  | Message 5 |  | – |
| 0x17B | Messages | Message text 5 | configuration | read/write |  | BMS: |  | – |
| 0x17C | Messages | Message text 5 | configuration | read/write |  | <10% Capacity! |  | – |
| 0x17D | Messages | Message text 5 | configuration | read/write |  | Use VITAL loads! |  | – |
| 0x17E | Messages | Message 6 | configuration | read/write |  | Message 6 |  | – |
| 0x17F | Messages | Message text 6 | configuration | read/write |  | – |  | – |
| 0x180 | Messages | Message text 6 | configuration | read/write |  | – |  | – |
| 0x181 | Messages | Message text 6 | configuration | read/write |  | – |  | – |
| 0x10E | Events | Event 1 source | configuration | read/write |  | BOILER | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x10F | Events | Event 1 target | configuration | read/write |  | 11 |  | – |
| 0x110 | Events | Event 1 command | configuration | read/write |  | 0 |  | – |
| 0x111 | Events | Event 1 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x112 | Events | Event 2 source | configuration | read/write |  | SAFETY | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x113 | Events | Event 2 target | configuration | read/write |  | 6 |  | – |
| 0x114 | Events | Event 2 command | configuration | read/write |  | 1 |  | – |
| 0x115 | Events | Event 2 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x116 | Events | Event 3 source | configuration | read/write |  | SAFETY | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x117 | Events | Event 3 target | configuration | read/write |  | 6 |  | – |
| 0x118 | Events | Event 3 command | configuration | read/write |  | 5 |  | – |
| 0x119 | Events | Event 3 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x11A | Events | Event 4 source | configuration | read/write |  | MONITOR NAV | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x11B | Events | Event 4 target | configuration | read/write |  | 8 |  | – |
| 0x11C | Events | Event 4 command | configuration | read/write |  | 0 |  | – |
| 0x11D | Events | Event 4 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x11E | Events | Event 5 source | configuration | read/write |  | MONITOR NAV | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x11F | Events | Event 5 target | configuration | read/write |  | 4 |  | – |
| 0x120 | Events | Event 5 command | configuration | read/write |  | 0 |  | – |
| 0x121 | Events | Event 5 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x122 | Events | Event 6 source | configuration | read/write |  | MONITOR NAV | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x123 | Events | Event 6 target | configuration | read/write |  | 4 |  | – |
| 0x124 | Events | Event 6 command | configuration | read/write |  | 2 |  | – |
| 0x125 | Events | Event 6 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x126 | Events | Event 7 source | configuration | read/write |  | MONITOR NAV | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x127 | Events | Event 7 target | configuration | read/write |  | 4 |  | – |
| 0x128 | Events | Event 7 command | configuration | read/write |  | 3 |  | – |
| 0x129 | Events | Event 7 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x12A | Events | Event 8 source | configuration | read/write |  | MONITOR NAV | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x12B | Events | Event 8 target | configuration | read/write |  | 8 |  | – |
| 0x12C | Events | Event 8 command | configuration | read/write |  | 0 |  | – |
| 0x12D | Events | Event 8 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x12E | Events | Event 9 source | configuration | read/write |  | SIGNALK | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x12F | Events | Event 9 target | configuration | read/write |  | 4 |  | – |
| 0x130 | Events | Event 9 command | configuration | read/write |  | 4 |  | – |
| 0x131 | Events | Event 9 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x132 | Events | Event 10 source | configuration | read/write |  | CHARGERS | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x133 | Events | Event 10 target | configuration | read/write |  | 7 |  | – |
| 0x134 | Events | Event 10 command | configuration | read/write |  | 0 |  | – |
| 0x135 | Events | Event 10 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x136 | Events | Event 11 source | configuration | read/write |  | CHARGERS | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x137 | Events | Event 11 target | configuration | read/write |  | 12 |  | – |
| 0x138 | Events | Event 11 command | configuration | read/write |  | 1 |  | – |
| 0x139 | Events | Event 11 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x13A | Events | Event 12 source | configuration | read/write |  | LAN ROUTER | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |
| 0x13B | Events | Event 12 target | configuration | read/write |  | 4 |  | – |
| 0x13C | Events | Event 12 command | configuration | read/write |  | 6 |  | – |
| 0x13D | Events | Event 12 data | configuration | read/write |  | Toggle | Off, On, Copy, Copy invert, Toggle | – |
| 0x13E | Events | Event 13 source | configuration | read/write |  | Disabled | Disabled, BOILER, CHARGERS, SAFETY, MONITOR NAV, LAN ROUTER, SIGNALK, State | – |

## MAC 1 BOW

Article 81205100, firmware 1.8, serial KD20D3409, Signal K instance `1Bow`, menus: monitoring, alarm, configuration, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Status | Device state | monitoring | read |  | Charging | Standby, Go to sleep, Charging, Alarm | `electrical.chargers.1Bow.deviceMode` |
| 0x001 | Status | Charge state | monitoring | read |  | Float | Off, Initialize bulk, Bulk, Absorption, Float, Constant voltage, Stopped | `electrical.chargers.1Bow.chargingMode` |
| 0x040 | Status | On/Standby | monitoring | read/write |  | True |  | `electrical.chargers.1Bow.enabled` (put) |
| 0x004 | DC in | Input voltage | monitoring | read | V | 13.468328 |  | `electrical.chargers.1Bow.input.voltage` |
| 0x005 | DC in | Input current | monitoring | read | A | 0.08894567 |  | `electrical.chargers.1Bow.input.current` |
| 0x006 | DC out | Output voltage | monitoring | read | V | 14.277734 |  | `electrical.chargers.1Bow.voltage` |
| 0x007 | DC out | Output current | monitoring | read | A | 0.083918564 |  | `electrical.chargers.1Bow.current` |
| 0x008 | DC out | Bat. volt sense | monitoring | read | V | 14.322967 |  | `electrical.chargers.1Bow.voltageSense` |
| 0x009 | Remote | Remote input | monitoring | read |  | 0 |  | `electrical.chargers.1Bow.remote.remoteInput` |
| 0x00E | Temperature | Battery | monitoring | read | °C | 9.0 |  | `electrical.chargers.1Bow.battery.temperature` |
| 0x200 | Alarm status | Dev. temp. high | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.devTempHigh` |
| 0x201 | Alarm status | Bat. temp. high | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.batTempHigh` |
| 0x202 | Alarm status | Bat. temp. low | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.batTempLow` |
| 0x203 | Alarm status | Input high | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.inputHigh` |
| 0x204 | Alarm status | Input low | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.inputLow` |
| 0x205 | Alarm status | Output high | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.outputHigh` |
| 0x206 | Alarm status | Output low | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.outputLow` |
| 0x207 | Alarm status | Over volt. prot. | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.overVoltProt` |
| 0x20A | Alarm status | Over curr. prot. | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.overCurrProt` |
| 0x208 | Alarm status | HW fault | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.hwFault` |
| 0x209 | Alarm status | Cable loss high | alarm | read |  | 0 |  | `electrical.chargers.1Bow.alarmStatus.cableLossHigh` |
| 0x002 | Outputs | Standby | monitoring | event output |  | – |  | – |
| 0x02B | Outputs | Bulk | monitoring | event output |  | – |  | – |
| 0x02D | Outputs | Absorption | monitoring | event output |  | – |  | – |
| 0x02F | Outputs | Float | monitoring | event output |  | – |  | – |
| 0x03E | Outputs | Stop charge | monitoring | event output |  | – |  | – |
| 0x100 | Device | Name | configuration | read/write |  | 1 BOW |  | – |
| 0x13E | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Francais, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x101 | Device | Serial number | configuration | read |  | KD20D3409 |  | – |
| 0x126 | Device | DIP switches | configuration | read |  | 11000000 |  | – |
| 0x146 | Processor 1 | Version | configuration | read |  | 1.8.0 |  | – |
| 0x147 | Processor 1 | Revision | configuration | read |  | AD0108 |  | – |
| 0x140 | Processor 1 | Bootloader ver. | configuration | read |  | 0.55.0 |  | – |
| 0x103 | Remote input | Mode | configuration | read/write |  | Not used | Not used, Active low, Active high | – |
| 0x104 | Input threshold | Enabled | configuration | read/write |  | 0 |  | – |
| 0x105 | Input threshold | Enable voltage | configuration | read/write | V | 12.5 |  | – |
| 0x106 | Input threshold | Enable delay | configuration | read/write | sec. | 2.0 |  | – |
| 0x107 | Input threshold | Disable voltage | configuration | read/write | V | 12.0 |  | – |
| 0x108 | Input threshold | Disable delay | configuration | read/write | sec. | 300.0 |  | – |
| 0x109 | Input threshold | Instant disable | configuration | read/write | V | 11.0 |  | – |
| 0x10A | Input threshold | Sleep delay | configuration | read/write | sec. | 300.0 |  | – |
| 0x128 | Charger | Method | configuration | read/write |  | 3-Step+ (IUoU) | 3-Step+ (IUoU), Constant voltage | – |
| 0x10B | Charger | Battery type | configuration | read/write |  | AGM | Flooded, Gel, AGM, Spiral, Lithium-ion, Traction, NiCad, User defined | – |
| 0x10C | Charger | Max output | configuration | read/write | A | 40.0 |  | – |
| 0x10D | Charger | Max input | configuration | read | A | 50.0 |  | – |
| 0x10E | Charger | Current ramp up | configuration | read | A/sec. | 5.0 |  | – |
| 0x10F | Charger | Temp. compensate | configuration | read | V/°C | -0.03 |  | – |
| 0x148 | Charger | CV Power Control | configuration | read/write |  | 0 |  | – |
| 0x110 | Bulk | Bulk voltage | configuration | read | V | 14.25 |  | – |
| 0x111 | Bulk | Max bulk time | configuration | read | Min | 480.00095 |  | – |
| 0x112 | Bulk | Min bulk time | configuration | read | Min | 2.0000038 |  | – |
| 0x113 | Bulk | Start bulk time | configuration | read | V | 13.25 |  | – |
| 0x116 | Absorption | Abs. voltage | configuration | read | V | 14.25 |  | – |
| 0x117 | Absorption | Max absorp. time | configuration | read | Min | 240.00047 |  | – |
| 0x118 | Absorption | Min absorp. time | configuration | read | Min | 15.00003 |  | – |
| 0x119 | Absorption | Return amps | configuration | read | % | 6.0 |  | – |
| 0x11A | Float | Float voltage | configuration | read | V | 13.8 |  | – |
| 0x114 | Float | Bulk ret. volt | configuration | read | V | 12.8 |  | – |
| 0x115 | Float | Bulk return time | configuration | read | sec. | 30.0 |  | – |
| 0x11C | Input alarm | High alarm on | configuration | read/write | V | 16.0 |  | – |
| 0x11D | Input alarm | High alarm off | configuration | read/write | V | 15.5 |  | – |
| 0x11E | Input alarm | Low alarm off | configuration | read/write | V | 11.0 |  | – |
| 0x11F | Input alarm | Low alarm on | configuration | read/write | V | 10.0 |  | – |
| 0x120 | Input alarm | Low alarm delay | configuration | read/write | sec. | 5.0 |  | – |
| 0x121 | Output alarm | High alarm on | configuration | read/write | V | 15.25 |  | – |
| 0x122 | Output alarm | High alarm off | configuration | read/write | V | 14.75 |  | – |
| 0x123 | Output alarm | Low alarm off | configuration | read/write | V | 11.0 |  | – |
| 0x124 | Output alarm | Low alarm on | configuration | read/write | V | 10.0 |  | – |
| 0x125 | Output alarm | Low alarm delay | configuration | read/write | sec. | 30.0 |  | – |
| 0x14A | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, Standby, Bulk, Absorption, Float, Alarm | – |

## MAC 3 VITAL

Article 81205105, firmware 1.8, serial S702A4687, Signal K instance `3Vital`, menus: monitoring, alarm, configuration, history.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | Status | Device state | monitoring | read |  | Charging | Standby, Go to sleep, Charging, Alarm | `electrical.chargers.3Vital.deviceMode` |
| 0x001 | Status | Charge state | monitoring | read |  | Float | Off, Initialize bulk, Bulk, Absorption, Float, Constant voltage, Stopped | `electrical.chargers.3Vital.chargingMode` |
| 0x040 | Status | On/Standby | monitoring | read/write |  | True |  | `electrical.chargers.3Vital.enabled` (put) |
| 0x004 | DC in | Input voltage | monitoring | read | V | 13.479237 |  | `electrical.chargers.3Vital.input.voltage` |
| 0x005 | DC in | Input current | monitoring | read | A | 5.8958244 |  | `electrical.chargers.3Vital.input.current` |
| 0x006 | DC out | Output voltage | monitoring | read | V | 14.371151 |  | `electrical.chargers.3Vital.voltage` |
| 0x007 | DC out | Output current | monitoring | read | A | 5.9397907 |  | `electrical.chargers.3Vital.current` |
| 0x008 | DC out | Bat. volt sense | monitoring | read | V | 14.334645 |  | `electrical.chargers.3Vital.voltageSense` |
| 0x009 | Remote | Remote input | monitoring | read |  | 0 |  | `electrical.chargers.3Vital.remote.remoteInput` |
| 0x00E | Temperature | Battery | monitoring | read | °C | 7.0 |  | `electrical.chargers.3Vital.battery.temperature` |
| 0x200 | Alarm status | Dev. temp. high | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.devTempHigh` |
| 0x201 | Alarm status | Bat. temp. high | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.batTempHigh` |
| 0x202 | Alarm status | Bat. temp. low | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.batTempLow` |
| 0x203 | Alarm status | Input high | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.inputHigh` |
| 0x204 | Alarm status | Input low | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.inputLow` |
| 0x205 | Alarm status | Output high | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.outputHigh` |
| 0x206 | Alarm status | Output low | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.outputLow` |
| 0x207 | Alarm status | Over volt. prot. | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.overVoltProt` |
| 0x20A | Alarm status | Over curr. prot. | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.overCurrProt` |
| 0x208 | Alarm status | HW fault | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.hwFault` |
| 0x209 | Alarm status | Cable loss high | alarm | read |  | 0 |  | `electrical.chargers.3Vital.alarmStatus.cableLossHigh` |
| 0x002 | Outputs | Standby | monitoring | event output |  | – |  | – |
| 0x02B | Outputs | Bulk | monitoring | event output |  | – |  | – |
| 0x02D | Outputs | Absorption | monitoring | event output |  | – |  | – |
| 0x02F | Outputs | Float | monitoring | event output |  | – |  | – |
| 0x03E | Outputs | Stop charge | monitoring | event output |  | – |  | – |
| 0x100 | Device | Name | configuration | read/write |  | 3 VITAL |  | – |
| 0x13E | Device | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Francais, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x101 | Device | Serial number | configuration | read |  | S702A4687 |  | – |
| 0x127 | Device | CZone enabled | configuration | read |  | 0 |  | – |
| 0x126 | Device | DIP switches | configuration | read |  | 11 00000000 |  | – |
| 0x146 | Processor 1 | Version | configuration | read |  | 1.8.0 |  | – |
| 0x147 | Processor 1 | Revision | configuration | read |  | AB0108 |  | – |
| 0x140 | Processor 1 | Bootloader ver. | configuration | read |  | 1.2.0 |  | – |
| 0x103 | Remote input | Mode | configuration | read/write |  | Not used | Not used, Active low, Active high | – |
| 0x104 | Input threshold | Enabled | configuration | read/write |  | 0 |  | – |
| 0x105 | Input threshold | Enable voltage | configuration | read/write | V | 12.5 |  | – |
| 0x106 | Input threshold | Enable delay | configuration | read/write | sec. | 2.0 |  | – |
| 0x107 | Input threshold | Disable voltage | configuration | read/write | V | 12.0 |  | – |
| 0x108 | Input threshold | Disable delay | configuration | read/write | sec. | 300.0 |  | – |
| 0x109 | Input threshold | Instant disable | configuration | read/write | V | 11.0 |  | – |
| 0x10A | Input threshold | Sleep delay | configuration | read/write | sec. | 300.0 |  | – |
| 0x128 | Charger | Method | configuration | read/write |  | 3-Step+ (IUoU) | 3-Step+ (IUoU), Constant voltage | – |
| 0x10B | Charger | Battery type | configuration | read/write |  | AGM | Flooded, Gel, AGM, Lithium-ion, User defined | – |
| 0x10C | Charger | Max output | configuration | read/write | A | 40.0 |  | – |
| 0x10D | Charger | Max input | configuration | read | A | 50.0 |  | – |
| 0x10E | Charger | Current ramp up | configuration | read | A/sec. | 5.0 |  | – |
| 0x10F | Charger | Temp. compensate | configuration | read | V/°C | -0.03 |  | – |
| 0x148 | Charger | CV Power Control | configuration | read/write |  | 0 |  | – |
| 0x110 | Bulk | Bulk voltage | configuration | read | V | 14.25 |  | – |
| 0x111 | Bulk | Max bulk time | configuration | read | Min | 480.00095 |  | – |
| 0x112 | Bulk | Min bulk time | configuration | read | Min | 2.0000038 |  | – |
| 0x113 | Bulk | Start bulk time | configuration | read | V | 13.25 |  | – |
| 0x116 | Absorption | Abs. voltage | configuration | read | V | 14.25 |  | – |
| 0x117 | Absorption | Max absorp. time | configuration | read | Min | 240.00047 |  | – |
| 0x118 | Absorption | Min absorp. time | configuration | read | Min | 15.00003 |  | – |
| 0x119 | Absorption | Return amps | configuration | read | % | 6.0 |  | – |
| 0x11A | Float | Float voltage | configuration | read | V | 13.8 |  | – |
| 0x114 | Float | Bulk ret. volt | configuration | read | V | 12.8 |  | – |
| 0x115 | Float | Bulk return time | configuration | read | sec. | 30.0 |  | – |
| 0x11C | Input alarm | High alarm on | configuration | read/write | V | 16.0 |  | – |
| 0x11D | Input alarm | High alarm off | configuration | read/write | V | 15.5 |  | – |
| 0x11E | Input alarm | Low alarm off | configuration | read/write | V | 11.0 |  | – |
| 0x11F | Input alarm | Low alarm on | configuration | read/write | V | 10.0 |  | – |
| 0x120 | Input alarm | Low alarm delay | configuration | read/write | sec. | 5.0 |  | – |
| 0x121 | Output alarm | High alarm on | configuration | read/write | V | 15.25 |  | – |
| 0x122 | Output alarm | High alarm off | configuration | read/write | V | 14.75 |  | – |
| 0x123 | Output alarm | Low alarm off | configuration | read/write | V | 11.0 |  | – |
| 0x124 | Output alarm | Low alarm on | configuration | read/write | V | 10.0 |  | – |
| 0x125 | Output alarm | Low alarm delay | configuration | read/write | sec. | 30.0 |  | – |
| 0x14A | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, Standby, Bulk, Absorption, Float, Alarm | – |

## ISO GI Trafo

Article 88000355, firmware 1.7, serial L807M6200, Signal K instance `giTrafo`, menus: monitoring, alarm, history, configuration.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x00D | State | State | monitoring | read |  | On | No shore, Standby, Waiting, Short circuit, On, Load high, Overload, Overload shutdwn, Config fault | `electrical.ac.giTrafo.state` |
| 0x007 | State | Shore fuse | monitoring | read/write |  | 10A | 2A, 4A, 6A, 10A, 14A, 16A, 32A, 50A, 63A, Unlimited | `electrical.ac.giTrafo.shoreFuse` (put) |
| 0x01C | State | Reset | monitoring | event output |  | – |  | – |
| 0x003 | Output | Output | monitoring | read | V | 229.5 |  | `electrical.ac.giTrafo.phase.single.lineNeutralVoltage` |
| 0x004 | Output | Output | monitoring | read | A | 4.72 |  | `electrical.ac.giTrafo.phase.single.current` |
| 0x01E | Output | Output | monitoring | read | kW | 1.1581979 |  | `electrical.ac.giTrafo.phase.single.realPower` |
| 0x01F | Shore | cos phi | monitoring | read |  | 0.99739856 |  | `electrical.ac.shore.phase.single.powerFactor` |
| 0x002 | Shore | Shore | monitoring | read | V | 226.04 |  | `electrical.ac.shore.phase.single.lineNeutralVoltage` |
| 0x009 | Shore | Shore | monitoring | read | A | 5.2430005 |  | `electrical.ac.shore.phase.single.current` |
| 0x018 | Shore | Shore | monitoring | read | kW | 1.170198 |  | `electrical.ac.shore.phase.single.realPower` |
| 0x001 | Shore | Shore | monitoring | read | Hz | 50.0 |  | `electrical.ac.shore.phase.single.frequency` |
| 0x202 | Alarm | Voltage high | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.voltageHigh` |
| 0x203 | Alarm | Temperature high | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.temperatureHigh` |
| 0x201 | Alarm | Short circuit | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.shortCircuit` |
| 0x200 | Alarm | Overload shutdwn | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.overloadShutdwn` |
| 0x204 | Alarm | Low voltage | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.lowVoltage` |
| 0x205 | Alarm | Frequency fail | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.frequencyFail` |
| 0x206 | Alarm | Shore fuse >80% | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.shoreFuse80` |
| 0x207 | Alarm | Shore fuse >100% | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.shoreFuse100` |
| 0x208 | Alarm | Shore fuse >120% | alarm | read |  | 0 |  | `electrical.ac.giTrafo.alarm.shoreFuse120` |
| 0x304 | Shore | Shore | history | read |  | 1011801.0 |  | `electrical.ac.giTrafo.shore.shore` |
| 0x305 | Shore | Energy | history | read | kWh | 206.45801 |  | `electrical.ac.giTrafo.shore.energy` |
| 0x306 | Shore | Maximum | history | read | A | 15.92 |  | `electrical.ac.giTrafo.shore.maximum.current` |
| 0x307 | Shore | Average | history | read | A | 3.5476067 |  | `electrical.ac.giTrafo.shore.average.current` |
| 0x309 | Shore | Maximum | history | read | V | 238.06999 |  | `electrical.ac.giTrafo.shore.maximum.voltage` |
| 0x30B | Shore | Average | history | read | V | 15.561768 |  | `electrical.ac.giTrafo.shore.average.voltage` |
| 0x30A | Shore | Minimum | history | read | V | 210.31 |  | `electrical.ac.giTrafo.shore.minimum.voltage` |
| 0x30C | Shore | Reset | history | read/write |  | False |  | `electrical.ac.giTrafo.shore.reset` |
| 0x308 | No shore | No shore | history | read |  | 5800.0 |  | `electrical.ac.giTrafo.noShore.noShore` |
| 0x302 | Total | Run time | history | read |  | 171448336.0 |  | `electrical.ac.giTrafo.total.runTime` |
| 0x300 | Total | Energy | history | read | kWh | 17217.988 |  | `electrical.ac.giTrafo.total.energy` |
| 0x303 | Total | Average | history | read | A | 2.2966166 |  | `electrical.ac.giTrafo.total.average.current` |
| 0x30D | Latest alarms | Latest | history | read/write |  | 0.0 |  | `electrical.ac.giTrafo.latestAlarms.latest` |
| 0x30E | Latest alarms | Alarm | history | read |  | Shore fuse >100% | No failure, Short circuit, Overload shutdwn, Voltage high, Temperature high, Low voltage, Frequency fail, Shore fuse >120%, Shore fuse >100% | `electrical.ac.giTrafo.latestAlarms.alarm` |
| 0x30F | Latest alarms | Shore | history | read | V | 224.95 |  | `electrical.ac.giTrafo.latestAlarms.shore.voltage` |
| 0x310 | Latest alarms | Output | history | read | V | 225.18 |  | `electrical.ac.giTrafo.phase.single0x310.lineNeutralVoltage` |
| 0x311 | Latest alarms | Output | history | read | A | 10.0199995 |  | `electrical.ac.giTrafo.phase.single0x311.current` |
| 0x112 | General | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x111 | General | Device name | configuration | read/write |  | GI Trafo |  | – |
| 0x10E | Reset | Reset setup | configuration | read/write |  | False |  | – |
| 0x11C | Parallel | Parallel setting | configuration | read/write |  | Single device | Auto, Single device, Master | – |
| 0x11E | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, Output, LED: 5%, LED: 25%, LED: 50%, LED: 75%, LED: 100%, LED: Overload, MPC >80%, MPC >100%, MPC >120%, Fan on, Failure mode | – |

## MSH VITAL

Article 77020110, firmware 0.66, serial LD04F8466, Signal K instance `vital`, menus: monitoring, alarm, history, configuration.

| Id | Group | Field | Menu | Access | Unit | Value | Options | Signal K path |
|---|---|---|---|---|---|--:|---|---|
| 0x000 | General | State of Charge | monitoring | read | % | 100.0 |  | `electrical.batteries.vital.capacity.stateOfCharge` |
| 0x004 | General | Remaining | monitoring | read |  | – |  | `electrical.batteries.vital.capacity.timeRemaining` |
| 0x003 | General | Cap. consumed | monitoring | read | Ah | 0.0 |  | `electrical.batteries.vital.capacity.dischargeSinceFull` |
| 0x001 | General | Battery | monitoring | read | V | 14.299581 |  | `electrical.batteries.vital.voltage` |
| 0x002 | General | Battery | monitoring | read | A | 0.16689402 |  | `electrical.batteries.vital.current` |
| 0x005 | General | Battery | monitoring | read | °C | 8.0 |  | `electrical.batteries.vital.temperature` |
| 0x00A | General | Time | monitoring | read |  | 39981.0 |  | `electrical.batteries.vital.general.time` |
| 0x00B | General | Date | monitoring | read |  | 2026-11-19 |  | `electrical.batteries.vital.general.date` |
| 0x200 | Alarm | Voltage high | alarm | read |  | 0 |  | `electrical.batteries.vital.alarm.voltageHigh` |
| 0x202 | Alarm | Very low voltage | alarm | read |  | 0 |  | `electrical.batteries.vital.alarm.veryLowVoltage` |
| 0x206 | Alarm | Very low battery | alarm | read |  | 0 |  | `electrical.batteries.vital.alarm.veryLowBattery` |
| 0x203 | Alarm | Fuse alarm | alarm | read |  | 0 |  | `electrical.batteries.vital.alarm.fuseAlarm` |
| 0x201 | Warning | Low voltage | alarm | read |  | 0 |  | `electrical.batteries.vital.warning.lowVoltage` |
| 0x205 | Warning | Low battery | alarm | read |  | 0 |  | `electrical.batteries.vital.warning.lowBattery` |
| 0x300 | Standard history | Days running | history | read |  | 58798812.0 |  | `electrical.batteries.vital.standardHistory.daysRunning` |
| 0x303 | Standard history | Last time 100% | history | read |  | 0.0 |  | `electrical.batteries.vital.standardHistory.lastTime100` |
| 0x301 | Standard history | Battery usage | history | read | cycles | 0.0 |  | `electrical.batteries.vital.standardHistory.batteryUsage` |
| 0x302 | Standard history | Battery abuse | history | read | times | 1.0 |  | `electrical.batteries.vital.standardHistory.batteryAbuse` |
| 0x304 | Standard history | Discharged state | history | read |  | 0.0 |  | `electrical.batteries.vital.standardHistory.dischargedState` |
| 0x305 | Standard history | Discharged state | history | read | times | 0.0 |  | `electrical.batteries.vital.standardHistory.dischargedState0x305` |
| 0x306 | Standard history | Calculated CEF | history | read | % | 94.0 |  | `electrical.batteries.vital.standardHistory.calculatedCef` |
| 0x30B | Restarts | Setup resets | history | read | times | 0.0 |  | `electrical.batteries.vital.restarts.setupResets` |
| 0x30C | Restarts | Power cycles | history | read | times | 19.0 |  | `electrical.batteries.vital.restarts.powerCycles` |
| 0x307 | Min/Max history | Deepest disch. | history | read | Ah | 11.432526 |  | `electrical.batteries.vital.minMaxHistory.deepestDisch` |
| 0x308 | Min/Max history | U @ deepest dis. | history | read | V | 12.758979 |  | `electrical.batteries.vital.minMaxHistory.uDeepestDis.voltage` |
| 0x309 | Min/Max history | Highest voltage | history | read | V | 14.841792 |  | `electrical.batteries.vital.minMaxHistory.highestVoltage.voltage` |
| 0x30A | Min/Max history | Lowest voltage | history | read | V | 12.054704 |  | `electrical.batteries.vital.minMaxHistory.lowestVoltage.voltage` |
| 0x30D | Alarm timers | Voltage high | history | read |  | 0.0 |  | `electrical.batteries.vital.alarmTimers.voltageHigh` |
| 0x30E | Alarm timers | Very low voltage | history | read |  | 0.0 |  | `electrical.batteries.vital.alarmTimers.veryLowVoltage` |
| 0x30F | Alarm timers | Very low battery | history | read |  | 2.0 |  | `electrical.batteries.vital.alarmTimers.veryLowBattery` |
| 0x310 | Warning timers | Low voltage | history | read |  | 0.0 |  | `electrical.batteries.vital.warningTimers.lowVoltage` |
| 0x311 | Warning timers | Low battery | history | read |  | 2.0 |  | `electrical.batteries.vital.warningTimers.lowBattery` |
| 0x103 | General | Language | configuration | read/write |  | English | English, Nederlands, Deutsch, Français, Castellano, Italiano, Norsk, Svenska, Suomi, Dansk | – |
| 0x10B | General | Name | configuration | read/write |  | VITAL |  | – |
| 0x10D | General | Year | configuration | read/write |  | 2026.0 |  | – |
| 0x10E | General | Month | configuration | read/write |  | November | January, February, March, April, May, June, July, August, September, October, November, December | – |
| 0x10F | General | Day | configuration | read/write |  | 18.0 |  | – |
| 0x110 | General | Hour | configuration | read/write |  | 16.0 |  | – |
| 0x111 | General | Minute | configuration | read/write |  | 55.0 |  | – |
| 0x10C | General | MasterBus power | configuration | read/write |  | Always on | Off, Smart on, Always on | – |
| 0x101 | Battery | Battery capacity | configuration | read | Ah | 160.0 |  | – |
| 0x102 | Battery | Battery type | configuration | read |  | AGM | Flooded, Gel, AGM, MLI | – |
| 0x100 | Battery | Nominal voltage | configuration | read | V | 12.0 |  | – |
| 0x107 | Battery | Peukert | configuration | read/write |  | 1.27 |  | – |
| 0x104 | Battery | Amp=full | configuration | read/write | % | 2.0 |  | – |
| 0x105 | Battery | Float | configuration | read/write | V | 13.8 |  | – |
| 0x106 | Battery | Absorption | configuration | read/write | V | 14.25 |  | – |
| 0x108 | Battery | Reset setup will | configuration | read |  | void warranty | void warranty | – |
| 0x11D | Battery | Reset setup | configuration | read/write |  | False |  | – |
| 0x119 | Alarm voltages | Voltage high | configuration | read/write | V | 15.0 |  | – |
| 0x11A | Alarm voltages | High delay | configuration | read/write | s | 5.0 |  | – |
| 0x113 | Alarm voltages | Low voltage | configuration | read/write | V | 11.0 |  | – |
| 0x114 | Alarm voltages | Very low voltage | configuration | read/write | V | 10.0 |  | – |
| 0x118 | Alarm voltages | Low delay | configuration | read/write | s | 30.0 |  | – |
| 0x13B | Alarm voltages | Low alarm | configuration | read/write |  | 0 |  | – |
| 0x117 | Battery alarms | Battery full | configuration | read/write | % SOC | 90.0 |  | – |
| 0x115 | Battery alarms | Low battery | configuration | read/write | % SOC | 50.0 |  | – |
| 0x116 | Battery alarms | Very low battery | configuration | read/write | % SOC | 10.0 |  | – |
| 0x13C | Battery alarms | Low alarm | configuration | read/write |  | 0 |  | – |
| 0x137 | Temperature | Unit | configuration | read/write |  | Celsius | Celsius, Fahrenheit | – |
| 0x139 | Temperature | Level | configuration | read/write | °C | 40.0 |  | – |
| 0x11E | Timer 1 | Timer select | configuration | read/write |  | Disabled | Disabled, Sunday, Monday, Tuesday, Wednesday, Thursday, Friday, Saturday, Weekday, Weekend, Daily, Monthly | – |
| 0x11F | Timer 1 | Timer on | configuration | read/write | Hour | 0.0 |  | – |
| 0x120 | Timer 1 | Timer on | configuration | read/write | Minute | 0.0 |  | – |
| 0x121 | Timer 1 | Timer off | configuration | read/write | Hour | 0.0 |  | – |
| 0x122 | Timer 1 | Timer off | configuration | read/write | Minute | 0.0 |  | – |
| 0x123 | Timer 1 | Stop when full | configuration | read/write |  | 0 |  | – |
| 0x144 | Events | Event 1 source | configuration | read/write |  | Disabled | Disabled, Low voltage, Very low voltage, Low battery, Very low battery, Battery full, Voltage high, Charging, Battery > 20%, Battery > 40%, Battery > 60%, Battery > 80%, Battery 100%, Timer 1, Timer 2, Temperature | – |
| 0x124 | Timer 2 | Timer select | configuration | read/write |  | Disabled | Disabled, Sunday, Monday, Tuesday, Wednesday, Thursday, Friday, Saturday, Weekday, Weekend, Daily, Monthly | – |
| 0x125 | Timer 2 | Timer on | configuration | read/write | Hour | 0.0 |  | – |
| 0x126 | Timer 2 | Timer on | configuration | read/write | Minute | 0.0 |  | – |
| 0x127 | Timer 2 | Timer off | configuration | read/write | Hour | 0.0 |  | – |
| 0x128 | Timer 2 | Timer off | configuration | read/write | Minute | 0.0 |  | – |
| 0x129 | Timer 2 | Stop when full | configuration | read/write |  | 0 |  | – |
| 0x13D | Processor 1 | Version | configuration | read |  | 0.66.0 |  | – |
| 0x13E | Processor 1 | Revision | configuration | read |  | AA0066 |  | – |
