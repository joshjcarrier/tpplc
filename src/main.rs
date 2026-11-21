mod protocol;
mod transport;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pnet_datalink::NetworkInterface;
use pnet_datalink::{self as datalink, Channel, Config};
use protocol::{
    Adapter, PowerState, QosPriority, discover_devices, get_device, parse_mac, set_led,
    set_power_saving, set_qos,
};
use std::time::Duration;
use transport::Link;

#[derive(Debug, Parser)]
#[command(
    name = "tpplc",
    version,
    about = "Native HomePlug AV diagnostics and controls for TP-Link powerline adapters"
)]
struct Cli {
    /// Network interface used for HomePlug AV frames (for example en0).
    #[arg(short, long, global = true, default_value = "en0")]
    interface: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List network interfaces and their MAC addresses.
    Interfaces,

    /// Discover adapters and read their basic Broadcom identity.
    Discover {
        /// Listen this long for each discovery response.
        #[arg(long, default_value_t = 1500)]
        timeout_ms: u64,
    },

    /// Read adapter identity, link rates, and supported settings without writing.
    Get {
        /// Adapter MAC address.
        mac: String,
        /// Per-query response timeout.
        #[arg(long, default_value_t = 800)]
        timeout_ms: u64,
    },

    /// Change a supported adapter setting.
    Set {
        #[command(subcommand)]
        setting: Setting,
    },
}

#[derive(Debug, Subcommand)]
enum Setting {
    /// Enable or disable the adapter's LEDs.
    Led { mac: String, state: PowerState },
    /// Enable or disable power-saving standby.
    PowerSaving { mac: String, state: PowerState },
    /// Set the Broadcom QoS priority mapping.
    Qos { mac: String, priority: QosPriority },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command, Command::Interfaces) {
        return list_interfaces();
    }

    let device = datalink::interfaces()
        .into_iter()
        .find(|device| device.name == cli.interface)
        .with_context(|| format!("interface {:?} was not found", cli.interface))?;

    let config = Config {
        read_timeout: Some(Duration::from_millis(100)),
        write_timeout: Some(Duration::from_secs(2)),
        promiscuous: false,
        ..Config::default()
    };

    let (tx, rx) = match datalink::channel(&device, config).with_context(|| {
        format!(
            "cannot open {} for raw Ethernet access; on macOS, try running only this \
                 command with sudo",
            cli.interface
        )
    })? {
        Channel::Ethernet(tx, rx) => (tx, rx),
        _ => bail!(
            "{} does not provide an Ethernet data-link channel",
            cli.interface
        ),
    };

    let source = interface_mac(&device)
        .with_context(|| format!("could not read the MAC address for {}", cli.interface))?;
    let mut link = Link::new(source, tx, rx);

    match cli.command {
        Command::Interfaces => unreachable!(),
        Command::Discover { timeout_ms } => {
            let adapters = discover_devices(&mut link, Duration::from_millis(timeout_ms))?;
            print_adapters(&adapters);
        }
        Command::Get { mac, timeout_ms } => {
            let mac = parse_mac(&mac)?;
            get_device(&mut link, mac, Duration::from_millis(timeout_ms))?;
        }
        Command::Set { setting } => match setting {
            Setting::Led { mac, state } => {
                set_led(&mut link, parse_mac(&mac)?, state)?;
            }
            Setting::PowerSaving { mac, state } => {
                set_power_saving(&mut link, parse_mac(&mac)?, state)?;
            }
            Setting::Qos { mac, priority } => {
                set_qos(&mut link, parse_mac(&mac)?, priority)?;
            }
        },
    }

    Ok(())
}

fn list_interfaces() -> Result<()> {
    for device in datalink::interfaces() {
        let mac = interface_mac(&device)
            .map(protocol::format_mac)
            .unwrap_or_else(|| "-".to_owned());
        let state = if device.is_up() { "up" } else { "down" };
        println!("{:<12} {:<6} {}", device.name, state, mac);
    }
    Ok(())
}

fn interface_mac(device: &NetworkInterface) -> Option<[u8; 6]> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let output = Command::new("/usr/sbin/networksetup")
            .arg("-listallhardwareports")
            .output()
            .ok()?;
        if output.status.success()
            && let Some(mac) =
                parse_networksetup_mac(&String::from_utf8_lossy(&output.stdout), &device.name)
        {
            return Some(mac);
        }

        let output = Command::new("/sbin/ifconfig")
            .arg(&device.name)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut fields = line.split_whitespace();
            if fields.next() == Some("ether")
                && let Some(mac) = fields.next()
            {
                return protocol::parse_mac(mac).ok();
            }
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        device.mac.map(|mac| mac.octets())
    }
}

#[cfg(target_os = "macos")]
fn parse_networksetup_mac(output: &str, interface: &str) -> Option<[u8; 6]> {
    let mut current_device = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Device: ") {
            current_device = Some(value);
        } else if current_device == Some(interface)
            && let Some(value) = line.strip_prefix("Ethernet Address: ")
        {
            return protocol::parse_mac(value).ok();
        }
    }
    None
}

fn print_adapters(adapters: &[Adapter]) {
    if adapters.is_empty() {
        println!("No HomePlug AV adapters replied.");
        println!(
            "Check that this interface reaches the adapters' LAN and that the Wi-Fi AP \
             bridges local traffic (guest/client-isolated Wi-Fi will not work)."
        );
        return;
    }

    println!(
        "{:<17} {:<12} {:<32} User HFID / firmware",
        "MAC", "Chipset", "Manufacturer HFID"
    );
    for adapter in adapters {
        println!(
            "{:<17} {:<12} {:<32} {}",
            protocol::format_mac(adapter.mac),
            adapter.chipset.as_deref().unwrap_or("unknown"),
            adapter.manufacturer_hfid.as_deref().unwrap_or("-"),
            adapter.user_hfid.as_deref().unwrap_or("-"),
        );
        for station in &adapter.stations {
            println!(
                "  station {} (TEI {}, same network: {})",
                protocol::format_mac(station.mac),
                station.tei,
                station.same_network
            );
        }
        for rate in &adapter.link_rates {
            println!(
                "  link to {}: TX {} Mbps, RX {} Mbps",
                protocol::format_mac(rate.peer),
                rate.tx_mbps,
                rate.rx_mbps
            );
        }
    }
}
