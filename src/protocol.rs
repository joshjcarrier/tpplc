use crate::transport::Link;
use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use std::collections::BTreeMap;
use std::time::Duration;

const ETH_HPAV: u16 = 0x88e1;
const ETH_MEDIAXTREAM: u16 = 0x8912;
const HPAV_VERSION: u8 = 0x01;
const MX_VERSION: u8 = 0x02;
const MX_OUI: [u8; 3] = [0x00, 0x1f, 0x84];
const BROADCAST: [u8; 6] = [0xff; 6];

const CC_DISCOVER_LIST_REQ: u16 = 0x0014;
const CC_DISCOVER_LIST_CNF: u16 = 0x0015;
const MX_DISCOVER_REQ: u16 = 0xa070;
const MX_DISCOVER_CNF: u16 = 0xa071;
const MX_NW_STATS_REQ: u16 = 0xa02c;
const MX_NW_STATS_CNF: u16 = 0xa02d;
const MX_GET_PARAM_REQ: u16 = 0xa05c;
const MX_GET_PARAM_CNF: u16 = 0xa05d;
const MX_SET_PARAM_REQ: u16 = 0xa058;
const MX_SET_PARAM_CNF: u16 = 0xa059;
const MX_SET_PARAM_ALT_CNF: u16 = 0xa069;
const MX_APPLY_REQ: u16 = 0xa020;
const MX_APPLY_CNF: u16 = 0xa021;

const PARAM_MANUFACTURER_HFID: u16 = 0x0001;
const PARAM_USER_HFID: u16 = 0x0025;
const PARAM_POWER_STANDBY: u16 = 0x0029;
const PARAM_POWER_STANDBY_AUX: u16 = 0x0074;
const PARAM_LED_OPTIONS: u16 = 0x003f;
const PARAM_LED_AUX: u16 = 0x0095;
const PARAM_QOS_PRIORITY_MAP: u16 = 0x0069;
const QOS_CAP_OFFSETS: [usize; 8] = [2, 27, 52, 77, 102, 127, 152, 177];

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PowerState {
    On,
    Off,
}

impl PowerState {
    fn is_on(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum QosPriority {
    Internet,
    Gaming,
    AudioVideo,
    Voip,
}

impl QosPriority {
    fn cap_values(self) -> [u8; 8] {
        match self {
            Self::Internet => [0x18; 8],
            Self::AudioVideo => [0x58, 0x18, 0x18, 0x38, 0x58, 0x58, 0x78, 0x78],
            Self::Gaming => [0x38, 0x18, 0x18, 0x38, 0x58, 0x58, 0x78, 0x78],
            Self::Voip => [0x78, 0x18, 0x18, 0x38, 0x58, 0x58, 0x78, 0x78],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Station {
    pub mac: [u8; 6],
    pub tei: u8,
    pub same_network: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Adapter {
    pub mac: [u8; 6],
    pub stations: Vec<Station>,
    pub chipset: Option<String>,
    pub manufacturer_hfid: Option<String>,
    pub user_hfid: Option<String>,
    pub link_rates: Vec<LinkRate>,
}

#[derive(Debug, Clone)]
pub struct LinkRate {
    pub peer: [u8; 6],
    pub tx_mbps: u16,
    pub rx_mbps: u16,
}

pub fn parse_mac(value: &str) -> Result<[u8; 6]> {
    let bytes: Vec<u8> = value
        .split([':', '-'])
        .map(|part| u8::from_str_radix(part, 16))
        .collect::<std::result::Result<_, _>>()
        .context("MAC address must be six hexadecimal octets, e.g. 00:11:22:33:44:55")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("MAC address must contain exactly six octets"))
}

pub fn format_mac(mac: [u8; 6]) -> String {
    mac.iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn discover_devices(link: &mut Link, timeout: Duration) -> Result<Vec<Adapter>> {
    let mut adapters = BTreeMap::<[u8; 6], Adapter>::new();
    let frame = build_hpav_frame(BROADCAST, link.source, CC_DISCOVER_LIST_REQ, &[]);
    for response in link.request(&frame, ETH_HPAV, &[CC_DISCOVER_LIST_CNF], None, timeout)? {
        let src = source_mac(&response)?;
        adapters.entry(src).or_insert_with(|| Adapter {
            mac: src,
            ..Adapter::default()
        });
        for station in parse_discover_confirm(&response) {
            adapters
                .entry(station.mac)
                .or_insert_with(|| Adapter {
                    mac: station.mac,
                    ..Adapter::default()
                })
                .stations
                .push(station);
        }
    }

    // Broadcom MEDIAXTREAM discovery supplies a useful HFID/model hint.
    let frame = build_mx_frame(
        BROADCAST,
        link.source,
        MX_DISCOVER_REQ,
        next_sequence(),
        &[],
    );
    for response in link.request(&frame, ETH_MEDIAXTREAM, &[MX_DISCOVER_CNF], None, timeout)? {
        let src = source_mac(&response)?;
        let adapter = adapters.entry(src).or_insert_with(|| Adapter {
            mac: src,
            ..Adapter::default()
        });
        adapter.chipset = Some("Broadcom MEDIAXTREAM".to_owned());
        if let Some(hfid) = parse_mx_discover_hfid(&response) {
            adapter.manufacturer_hfid = Some(hfid);
        }
    }

    // Some TL-PA7017 firmware answers unicast NW_STATS but not the broadcast
    // MEDIAXTREAM Discover request. A unicast response identifies the chipset
    // and gives the link-rate rows shown by tpPLC.
    let macs: Vec<[u8; 6]> = adapters.keys().copied().collect();
    for mac in macs {
        let frame = build_mx_frame(mac, link.source, MX_NW_STATS_REQ, next_sequence(), &[]);
        let responses = link.request(
            &frame,
            ETH_MEDIAXTREAM,
            &[MX_NW_STATS_CNF],
            Some(mac),
            timeout,
        )?;
        if responses.is_empty() {
            continue;
        }

        let adapter = adapters.entry(mac).or_insert_with(|| Adapter {
            mac,
            ..Adapter::default()
        });
        adapter
            .chipset
            .get_or_insert_with(|| "Broadcom MEDIAXTREAM".to_owned());
        for response in responses {
            adapter.link_rates.extend(parse_mx_link_rates(&response)?);
        }
    }

    let macs: Vec<[u8; 6]> = adapters
        .values()
        .filter(|adapter| adapter.chipset.is_some())
        .map(|adapter| adapter.mac)
        .collect();
    for mac in macs {
        if let Some(value) = get_parameter(link, mac, PARAM_MANUFACTURER_HFID, timeout)?
            && let Some(adapter) = adapters.get_mut(&mac)
        {
            if let Some(hfid) = printable(&value) {
                adapter.manufacturer_hfid = Some(hfid);
            }
            adapter
                .chipset
                .get_or_insert_with(|| "Broadcom MEDIAXTREAM".to_owned());
        }
        if let Some(value) = get_parameter(link, mac, PARAM_USER_HFID, timeout)?
            && let Some(adapter) = adapters.get_mut(&mac)
        {
            adapter.user_hfid = printable(&value);
            adapter
                .chipset
                .get_or_insert_with(|| "Broadcom MEDIAXTREAM".to_owned());
        }
    }

    Ok(adapters.into_values().collect())
}

pub fn get_device(link: &mut Link, mac: [u8; 6], timeout: Duration) -> Result<()> {
    let frame = build_mx_frame(mac, link.source, MX_NW_STATS_REQ, next_sequence(), &[]);
    let responses = link.request(
        &frame,
        ETH_MEDIAXTREAM,
        &[MX_NW_STATS_CNF],
        Some(mac),
        timeout,
    )?;
    let manufacturer = get_parameter(link, mac, PARAM_MANUFACTURER_HFID, timeout)?;
    let firmware = get_parameter(link, mac, PARAM_USER_HFID, timeout)?;
    let led_options = get_parameter(link, mac, PARAM_LED_OPTIONS, timeout)?;
    let power_standby = get_parameter(link, mac, PARAM_POWER_STANDBY, timeout)?;
    let qos_table = get_parameter(link, mac, PARAM_QOS_PRIORITY_MAP, timeout)?;
    if responses.is_empty()
        && manufacturer.is_none()
        && firmware.is_none()
        && led_options.is_none()
        && power_standby.is_none()
        && qos_table.is_none()
    {
        bail!(
            "no supported Broadcom response from {}; confirm the MAC and interface",
            format_mac(mac)
        );
    }

    let rates = responses
        .iter()
        .map(|response| parse_mx_link_rates(response))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    println!("Adapter: {}", format_mac(mac));
    println!(
        "Chipset: {}",
        if !responses.is_empty() {
            "Broadcom MEDIAXTREAM"
        } else {
            "unknown"
        }
    );
    println!(
        "Manufacturer HFID: {}",
        manufacturer
            .as_deref()
            .and_then(printable)
            .as_deref()
            .unwrap_or("unknown")
    );
    println!(
        "Firmware / user HFID: {}",
        firmware
            .as_deref()
            .and_then(printable)
            .as_deref()
            .unwrap_or("unknown")
    );

    if responses.is_empty() {
        println!("Link rates: unavailable");
    } else if rates.is_empty() {
        println!("Link rates: no peer rate reported");
    } else {
        for rate in rates {
            println!(
                "Link to {}: TX {} Mbps, RX {} Mbps",
                format_mac(rate.peer),
                rate.tx_mbps,
                rate.rx_mbps
            );
        }
    }

    match decode_led_state(led_options.as_deref()) {
        Some(on) => println!("LEDs: {}", if on { "on" } else { "off" }),
        None => println!("LEDs: unknown"),
    }
    match decode_power_saving(power_standby.as_deref()) {
        Some((on, timeout)) => println!(
            "Power saving: {} (standby timeout: {timeout}s)",
            if on { "on" } else { "off" }
        ),
        None => println!("Power saving: unknown"),
    }
    match qos_table.as_deref().and_then(decode_qos_priority) {
        Some(priority) => println!("QoS priority: {priority}"),
        None => println!("QoS priority: unknown or custom"),
    }
    Ok(())
}

fn parse_mx_link_rates(frame: &[u8]) -> Result<Vec<LinkRate>> {
    let Some(payload) = frame.get(23..) else {
        return Ok(Vec::new());
    };
    let Some(&count) = payload.first() else {
        return Ok(Vec::new());
    };
    let responder = source_mac(frame)?;
    let mut rates = Vec::new();
    let mut offset = 1;
    for _ in 0..count {
        if offset + 10 > payload.len() {
            break;
        }
        let peer: [u8; 6] = payload[offset..offset + 6]
            .try_into()
            .expect("six-byte peer MAC");
        let tx_raw = u16::from_le_bytes([payload[offset + 6], payload[offset + 7]]);
        let rx_raw = u16::from_le_bytes([payload[offset + 8], payload[offset + 9]]);
        offset += 10;
        if peer == responder {
            continue; // The self entry is the adapter's rated capability.
        }
        rates.push(LinkRate {
            peer,
            tx_mbps: tx_raw & 0x07ff,
            rx_mbps: rx_raw & 0x07ff,
        });
    }
    Ok(rates)
}

pub fn set_led(link: &mut Link, mac: [u8; 6], state: PowerState) -> Result<()> {
    ensure_broadcom(link, mac)?;
    let on = state.is_on();

    set_parameter(
        link,
        mac,
        PARAM_LED_AUX,
        2,
        if on { &[0x00, 0x00] } else { &[0x47, 0x00] },
    )?;
    set_parameter(
        link,
        mac,
        PARAM_LED_OPTIONS,
        4,
        if on {
            &[0x02, 0xa0, 0x01, 0x12]
        } else {
            &[0x02, 0xa0, 0x01, 0x02]
        },
    )?;
    apply(link, mac)?;
    println!(
        "LEDs turned {} on {}.",
        if on { "on" } else { "off" },
        format_mac(mac)
    );
    Ok(())
}

pub fn set_power_saving(link: &mut Link, mac: [u8; 6], state: PowerState) -> Result<()> {
    ensure_broadcom(link, mac)?;
    let current = get_parameter(link, mac, PARAM_POWER_STANDBY, Duration::from_millis(900))?
        .context("could not read the standby timeout; refusing to change power saving")?;
    if current.len() < 2 {
        bail!("adapter returned an invalid standby timeout; refusing to write");
    }
    let timeout = u16::from_le_bytes([current[0], current[1]]) & 0x7fff;
    let value = timeout | if state.is_on() { 0x8000 } else { 0 };
    set_parameter(link, mac, PARAM_POWER_STANDBY, 2, &value.to_le_bytes())?;
    if !state.is_on() {
        set_parameter(link, mac, PARAM_POWER_STANDBY_AUX, 1, &[0])?;
    }
    apply(link, mac)?;
    println!(
        "Power saving turned {} on {} (standby timeout: {timeout}s).",
        if state.is_on() { "on" } else { "off" },
        format_mac(mac)
    );
    Ok(())
}

pub fn set_qos(link: &mut Link, mac: [u8; 6], priority: QosPriority) -> Result<()> {
    ensure_broadcom(link, mac)?;
    let mut table = get_parameter(link, mac, PARAM_QOS_PRIORITY_MAP, Duration::from_secs(2))?
        .context("adapter did not return its QoS table; refusing to overwrite it")?;
    let required = *QOS_CAP_OFFSETS.last().expect("non-empty offsets") + 1;
    if table.len() < required {
        bail!(
            "QoS table is only {} bytes (need at least {required}); refusing to write",
            table.len()
        );
    }
    for (offset, value) in QOS_CAP_OFFSETS.into_iter().zip(priority.cap_values()) {
        table[offset] = value;
    }
    set_parameter(link, mac, PARAM_QOS_PRIORITY_MAP, 1, &table)?;
    println!("QoS priority set to {priority:?} on {}.", format_mac(mac));
    Ok(())
}

fn ensure_broadcom(link: &mut Link, mac: [u8; 6]) -> Result<()> {
    // Some TL-PA7017 firmware does not return the manufacturer HFID parameter
    // even though it supports MEDIAXTREAM controls. The unicast NW_STATS
    // confirmation is the reliable Broadcom capability check on these units.
    let frame = build_mx_frame(mac, link.source, MX_NW_STATS_REQ, next_sequence(), &[]);
    let responses = link.request(
        &frame,
        ETH_MEDIAXTREAM,
        &[MX_NW_STATS_CNF],
        Some(mac),
        Duration::from_millis(1200),
    )?;
    if responses.is_empty() {
        bail!(
            "{} did not answer the Broadcom MEDIAXTREAM stats query; \
             no setting was changed",
            format_mac(mac)
        );
    }
    Ok(())
}

fn get_parameter(
    link: &mut Link,
    mac: [u8; 6],
    param_id: u16,
    timeout: Duration,
) -> Result<Option<Vec<u8>>> {
    let frame = build_mx_frame(
        mac,
        link.source,
        MX_GET_PARAM_REQ,
        next_sequence(),
        &param_id.to_le_bytes(),
    );
    let responses = link.request(
        &frame,
        ETH_MEDIAXTREAM,
        &[MX_GET_PARAM_CNF],
        Some(mac),
        timeout,
    )?;
    Ok(responses
        .iter()
        .find_map(|response| parse_parameter_value(response)))
}

fn set_parameter(
    link: &mut Link,
    mac: [u8; 6],
    param_id: u16,
    octets_per_element: u8,
    value: &[u8],
) -> Result<()> {
    if value.is_empty()
        || octets_per_element == 0
        || !value.len().is_multiple_of(octets_per_element as usize)
    {
        bail!("invalid Set Parameter value length");
    }
    let mut payload = Vec::with_capacity(5 + value.len());
    payload.extend_from_slice(&param_id.to_le_bytes());
    payload.push(octets_per_element);
    payload.extend_from_slice(&((value.len() / octets_per_element as usize) as u16).to_le_bytes());
    payload.extend_from_slice(value);

    let frame = build_mx_frame(
        mac,
        link.source,
        MX_SET_PARAM_REQ,
        next_sequence(),
        &payload,
    );
    let responses = link.request(
        &frame,
        ETH_MEDIAXTREAM,
        &[MX_SET_PARAM_CNF, MX_SET_PARAM_ALT_CNF],
        Some(mac),
        Duration::from_secs(2),
    )?;
    if responses.iter().any(|response| {
        matches!(
            mme_type(response),
            Some(MX_SET_PARAM_CNF | MX_SET_PARAM_ALT_CNF)
        )
    }) {
        Ok(())
    } else {
        bail!(
            "adapter {} did not confirm Set Parameter 0x{param_id:04X}",
            format_mac(mac)
        )
    }
}

fn apply(link: &mut Link, mac: [u8; 6]) -> Result<()> {
    let frame = build_mx_frame(mac, link.source, MX_APPLY_REQ, next_sequence(), &[]);
    let responses = link.request(
        &frame,
        ETH_MEDIAXTREAM,
        &[MX_APPLY_CNF],
        Some(mac),
        Duration::from_secs(2),
    )?;
    if responses.is_empty() {
        bail!("adapter {} did not confirm Apply", format_mac(mac));
    }
    Ok(())
}

fn parse_parameter_value(frame: &[u8]) -> Option<Vec<u8>> {
    let payload = frame.get(23..)?;
    if payload.len() < 3 {
        return None;
    }
    let octets_per_element = payload[0] as usize;
    let count = u16::from_le_bytes([payload[1], payload[2]]) as usize;
    if !matches!(octets_per_element, 1 | 2 | 4) || count == 0 {
        return None;
    }
    let length = octets_per_element.checked_mul(count)?;
    let end = 3usize.checked_add(length)?;
    (end <= payload.len()).then(|| payload[3..end].to_vec())
}

fn decode_led_state(value: Option<&[u8]>) -> Option<bool> {
    let value = value?;
    (value.len() >= 4).then(|| value[3] & 0x10 != 0)
}

fn decode_power_saving(value: Option<&[u8]>) -> Option<(bool, u16)> {
    let value = value?;
    if value.len() < 2 {
        return None;
    }
    let raw = u16::from_le_bytes([value[0], value[1]]);
    Some((raw & 0x8000 != 0, raw & 0x7fff))
}

fn decode_qos_priority(table: &[u8]) -> Option<&'static str> {
    let required = *QOS_CAP_OFFSETS.last()? + 1;
    if table.len() < required {
        return None;
    }
    [
        (QosPriority::Internet, "internet"),
        (QosPriority::Gaming, "gaming"),
        (QosPriority::AudioVideo, "audio-video"),
        (QosPriority::Voip, "voip"),
    ]
    .into_iter()
    .find_map(|(priority, name)| {
        QOS_CAP_OFFSETS
            .iter()
            .zip(priority.cap_values())
            .all(|(offset, expected)| table[*offset] == expected)
            .then_some(name)
    })
}

fn parse_discover_confirm(frame: &[u8]) -> Vec<Station> {
    let mut stations = Vec::new();
    let Some(payload) = frame.get(19..) else {
        return stations;
    };
    let Some(&count) = payload.first() else {
        return stations;
    };
    let mut offset = 1;
    for _ in 0..count {
        if offset + 12 > payload.len() {
            break;
        }
        let mac: [u8; 6] = payload[offset..offset + 6]
            .try_into()
            .expect("six-byte station MAC");
        stations.push(Station {
            mac,
            tei: payload[offset + 6],
            same_network: payload[offset + 7] == 1,
        });
        offset += 12;
    }
    stations
}

fn parse_mx_discover_hfid(frame: &[u8]) -> Option<String> {
    let payload = frame.get(23..)?;
    if payload.len() < 2 {
        return None;
    }
    let length = payload[1] as usize;
    let bytes = payload.get(2..2 + length)?;
    printable(bytes)
}

fn printable(bytes: &[u8]) -> Option<String> {
    let value = bytes
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .filter(|byte| byte.is_ascii_graphic() || *byte == b' ')
        .collect::<Vec<_>>();
    let value = String::from_utf8_lossy(&value).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn source_mac(frame: &[u8]) -> Result<[u8; 6]> {
    frame
        .get(6..12)
        .context("Ethernet frame is missing its source MAC")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid source MAC in Ethernet frame"))
}

fn mme_type(frame: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*frame.get(15)?, *frame.get(16)?]))
}

fn build_hpav_frame(dst: [u8; 6], src: [u8; 6], mmtype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(60.max(14 + 5 + payload.len()));
    frame.extend_from_slice(&dst);
    frame.extend_from_slice(&src);
    frame.extend_from_slice(&ETH_HPAV.to_be_bytes());
    frame.push(HPAV_VERSION);
    frame.extend_from_slice(&mmtype.to_le_bytes());
    frame.extend_from_slice(&[0, 0]); // Fragmentation information
    frame.extend_from_slice(payload);
    frame.resize(frame.len().max(60), 0);
    frame
}

fn build_mx_frame(
    dst: [u8; 6],
    src: [u8; 6],
    mmtype: u16,
    sequence: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(60.max(14 + 9 + payload.len()));
    frame.extend_from_slice(&dst);
    frame.extend_from_slice(&src);
    frame.extend_from_slice(&ETH_MEDIAXTREAM.to_be_bytes());
    frame.push(MX_VERSION);
    frame.extend_from_slice(&mmtype.to_le_bytes());
    frame.extend_from_slice(&[0, 0]); // Fragmentation information
    frame.extend_from_slice(&MX_OUI);
    frame.push(sequence);
    frame.extend_from_slice(payload);
    frame.resize(frame.len().max(60), 0);
    frame
}

fn next_sequence() -> u8 {
    use std::sync::atomic::{AtomicU8, Ordering};
    static SEQUENCE: AtomicU8 = AtomicU8::new(0);
    SEQUENCE.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mac_addresses() {
        assert_eq!(
            parse_mac("00:11:22:aa:bb:cc").unwrap(),
            [0, 0x11, 0x22, 0xaa, 0xbb, 0xcc]
        );
        assert!(parse_mac("00:11:22").is_err());
    }

    #[test]
    fn builds_standard_discovery_frame() {
        let frame = build_hpav_frame(BROADCAST, [1, 2, 3, 4, 5, 6], 0x0014, &[]);
        assert_eq!(frame.len(), 60);
        assert_eq!(&frame[12..14], &[0x88, 0xe1]);
        assert_eq!(&frame[14..19], &[0x01, 0x14, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn builds_mediaxtream_frame() {
        let frame = build_mx_frame(BROADCAST, [1, 2, 3, 4, 5, 6], 0xa058, 7, &[1, 2]);
        assert_eq!(&frame[12..14], &[0x89, 0x12]);
        assert_eq!(&frame[14..23], &[0x02, 0x58, 0xa0, 0, 0, 0, 0x1f, 0x84, 7]);
        assert_eq!(&frame[23..25], &[1, 2]);
    }

    #[test]
    fn decodes_broadcom_discovery_station_records() {
        let mut frame = build_hpav_frame([0xff; 6], [0; 6], CC_DISCOVER_LIST_CNF, &[]);
        frame.truncate(19);
        frame.push(1);
        frame.extend_from_slice(&[0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
        frame.extend_from_slice(&[3, 1]);
        frame.extend_from_slice(&[0; 4]);
        let stations = parse_discover_confirm(&frame);
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].mac, [0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
        assert_eq!(stations[0].tei, 3);
        assert!(stations[0].same_network);
    }

    #[test]
    fn parses_parameter_payload_and_qos_map() {
        let mut response = vec![0; 23];
        response.extend_from_slice(&[1, 3, 0, b'a', b'b', b'c']);
        assert_eq!(parse_parameter_value(&response).unwrap(), b"abc");

        let mut table = vec![0; 200];
        for (offset, value) in QOS_CAP_OFFSETS
            .into_iter()
            .zip(QosPriority::Gaming.cap_values())
        {
            table[offset] = value;
        }
        assert_eq!(table[2], 0x38);
        assert_eq!(table[177], 0x78);
    }

    #[test]
    fn parses_network_stats_and_skips_self_capability() {
        let responder = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let peer = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];
        let mut payload = vec![2];
        payload.extend_from_slice(&responder);
        payload.extend_from_slice(&1000u16.to_le_bytes());
        payload.extend_from_slice(&1000u16.to_le_bytes());
        payload.extend_from_slice(&peer);
        payload.extend_from_slice(&103u16.to_le_bytes());
        payload.extend_from_slice(&111u16.to_le_bytes());
        let frame = build_mx_frame([0; 6], responder, MX_NW_STATS_CNF, 1, &payload);

        let rates = parse_mx_link_rates(&frame).unwrap();
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].peer, peer);
        assert_eq!(rates[0].tx_mbps, 103);
        assert_eq!(rates[0].rx_mbps, 111);
    }

    #[test]
    fn decodes_read_only_settings() {
        assert_eq!(
            decode_led_state(Some(&[0x02, 0xa0, 0x01, 0x12])),
            Some(true)
        );
        assert_eq!(
            decode_led_state(Some(&[0x02, 0xa0, 0x01, 0x02])),
            Some(false)
        );
        assert_eq!(
            decode_power_saving(Some(&300u16.to_le_bytes())),
            Some((false, 300))
        );
        assert_eq!(
            decode_power_saving(Some(&(0x8000u16 | 300).to_le_bytes())),
            Some((true, 300))
        );

        let mut table = vec![0; 200];
        for (offset, value) in QOS_CAP_OFFSETS
            .into_iter()
            .zip(QosPriority::Gaming.cap_values())
        {
            table[offset] = value;
        }
        assert_eq!(decode_qos_priority(&table), Some("gaming"));
    }
}
