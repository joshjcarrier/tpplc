# tpplc

`tpplc` is a command-line tool for HomePlug AV powerline adapters.

The tool sends raw Ethernet frames. It does not use IP addresses to find or
manage adapters.

This is an independent project. TP-Link does not develop or support this
project.

## Features

- Find HomePlug AV adapters and show their MAC addresses.
- Read firmware, link rates, LED state, power-saving state, and QoS priority
  from supported Broadcom adapters.
- Change the LED, power-saving, and QoS settings on supported Broadcom
  adapters.

The rates from an adapter are PHY link rates. They are not application
throughput rates. Adapter models and firmware can report different data.

## Support

The vendor-specific commands support Broadcom MEDIAXTREAM adapters. Tests on
physical TP-Link AV1000 adapters confirm discovery, status reads, and
power-saving changes. LED and QoS write commands need more hardware tests.

Standard HomePlug discovery can find some adapters that do not support the
Broadcom commands. The tool does not support G.hn or other vendor-specific
management protocols.

The tool does not pair adapters, change network keys, reset adapters, or
upgrade firmware.

macOS is the tested build platform. Linux support is intended, but has not
been tested with physical adapters. Other operating systems are not supported.

## Requirements

- Rust 1.85 or later, stable release.
- A network interface on the same Layer 2 network as the adapters.
- Permission to open a raw Ethernet interface.

The tool uses HomePlug AV EtherType `0x88E1`. It uses Broadcom MEDIAXTREAM
EtherType `0x8912` for Broadcom adapters.

Wi-Fi can work if the access point bridges these frames. Guest networks,
client isolation, and some VLAN settings can block these frames.

macOS uses BPF for raw packet access. Run the command with `sudo` if your
account cannot open BPF. Linux needs root access or the required raw-packet
capability.

## Build

Build from the project directory. Do not run Cargo with `sudo`.

```sh
cargo build --release
```

The executable is `target/release/tpplc`.

The default network interface is `en0`. This is common on macOS. Set
`--interface` to the correct interface on other systems or when needed.

List network interfaces:

```sh
./target/release/tpplc interfaces
```

Example output:

```text
en0          up     00:11:22:33:44:55
en3          up     00:11:22:33:44:66
```

## Use

The examples use `en0`. Change the value if the adapters use another interface.
Raw packet access is needed for discovery, get, and set commands. Replace each
example MAC address with the MAC address of your adapter.

### Find adapters

```sh
sudo ./target/release/tpplc --interface en0 discover
```

Example output:

```text
MAC               Chipset      Manufacturer HFID                User HFID / firmware
02:00:00:00:00:01 Broadcom MEDIAXTREAM -                          tpver_701003_180102_901
  station 02:00:00:00:00:01 (TEI 13, same network: true)
  link to 02:00:00:00:00:02: TX 95 Mbps, RX 110 Mbps
02:00:00:00:00:02 Broadcom MEDIAXTREAM -                          tpver_701E04_190619_901
  station 02:00:00:00:00:02 (TEI 12, same network: true)
  link to 02:00:00:00:00:01: TX 110 Mbps, RX 95 Mbps
```

### Read adapter data

`get` only reads data. It does not change adapter settings.

```sh
sudo ./target/release/tpplc --interface en0 get 02:00:00:00:00:01
```

Example output:

```text
Adapter: 02:00:00:00:00:01
Chipset: Broadcom MEDIAXTREAM
Manufacturer HFID: unknown
Firmware / user HFID: tpver_701003_180102_901
Link to 02:00:00:00:00:02: TX 95 Mbps, RX 110 Mbps
LEDs: on
Power saving: off (standby timeout: 300s)
QoS priority: internet
```

### Change adapter settings

`set` changes the selected setting at once. Check the MAC address and setting
before you run the command. The tool has no dry-run mode or undo command.

```sh
sudo ./target/release/tpplc --interface en0 set led 02:00:00:00:00:01 off
```

Example output:

```text
LEDs turned off on 02:00:00:00:00:01.
```

```sh
sudo ./target/release/tpplc --interface en0 set power-saving 02:00:00:00:00:01 off
```

Example output:

```text
Power saving turned off on 02:00:00:00:00:01 (standby timeout: 300s).
```

```sh
sudo ./target/release/tpplc --interface en0 set qos 02:00:00:00:00:01 internet
```

Example output:

```text
QoS priority set to Internet on 02:00:00:00:00:01.
```

Run `tpplc --help` or `tpplc <command> --help` for all commands and options.

## Troubleshooting

- **The tool cannot open the interface:** run the command with `sudo` on
  macOS. On Linux, give the process raw-packet access.
- **The tool finds no adapters:** check the selected interface, power, cables,
  and HomePlug pairing. On Wi-Fi, check that the access point bridges Layer 2
  traffic. Do not use a guest network or client isolation.
- **Discovery finds an adapter but some fields show `unknown`:** the adapter
  can answer standard HomePlug discovery and not answer a Broadcom query.
- **No peer rate appears:** the adapter can be idle or unpaired. Some firmware
  does not report peer rates.

Do not use firmware for a different model, hardware version, or region. This
tool does not upgrade firmware.

## Develop

Run these checks before you submit a change:

```sh
cargo fmt --check
cargo test
cargo clippy -- -D warnings
cargo build --release
```

Read [CONTRIBUTING.md](CONTRIBUTING.md) for contribution and AI-use rules.

When you report a hardware problem, include the adapter model, hardware
version, firmware, operating system, interface name, command, and output.
Remove MAC addresses and unrelated network details from public reports.

Protocol values and behavior are based in part on
[Chance-Konstruktion/ha-powerline](https://github.com/Chance-Konstruktion/ha-powerline).
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for its license and
copyright notice. TP-Link's `plcu` utility was also used as a behavior
reference. This project does not include TP-Link software or that project's
Qualcomm PIB-write code.

## License

This project uses the MIT License. See [LICENSE](LICENSE).
