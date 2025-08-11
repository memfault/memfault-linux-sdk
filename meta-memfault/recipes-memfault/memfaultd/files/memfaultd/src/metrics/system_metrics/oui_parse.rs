//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs;
use std::path::Path;
use std::str::FromStr;

const SYS_CLASS_NET: &str = "/sys/class/net";
const PROC_NET_ARP: &str = "/proc/net/arp";

fn read_file_content(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn file_content_equals<T>(path: &Path, expected: T) -> bool
where
    T: PartialEq + FromStr,
    T::Err: std::fmt::Debug,
{
    read_file_content(path)
        .and_then(|content| content.parse().ok())
        .is_some_and(|value: T| value == expected)
}

pub fn get_active_wireless_local_ouis() -> Option<Vec<(String, String)>> {
    // https://www.man7.org/linux/man-pages/man5/sysfs.5.html
    let sys_net_path = Path::new(SYS_CLASS_NET);
    let mut ouis = Vec::new();

    let entries = fs::read_dir(sys_net_path).ok()?;

    for entry in entries.flatten() {
        let adapter_name = entry.file_name().to_string_lossy().to_string();
        let adapter_path = sys_net_path.join(&adapter_name);

        if is_wireless_adapter(&adapter_path) && is_adapter_active(&adapter_path) {
            if let Some(oui) =
                read_file_content(&adapter_path.join("address")).and_then(|mac| extract_oui(&mac))
            {
                ouis.push((adapter_name, oui));
            }
        }
    }

    Some(ouis)
}

/// Is adapter for wireless connections (ie, not loopback/local)?
/// Test /sys/class/net/$ADAPTER/type == 1 (link type is of Ethernet)
/// and /sys/class/net/$ADAPTER/wireless/ exists (adapter is wireless)
/// sources: https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-class-net,
/// https://github.com/torvalds/linux/blob/master/include/uapi/linux/if_arp.h
fn is_wireless_adapter(adapter_path: &Path) -> bool {
    file_content_equals(&adapter_path.join("type"), 1u32) && adapter_path.join("wireless").exists()
}

/// Is adapter currently in operation and connected?
/// Test /sys/class/net/$ADAPTER/operstate == up (interface RFC2863 operational state)
/// and /sys/class/net/$ADAPTER/carrier == 1 (physical link is up)
/// source: https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-class-net
fn is_adapter_active(adapter_path: &Path) -> bool {
    file_content_equals(&adapter_path.join("operstate"), "up".to_string())
        && file_content_equals(&adapter_path.join("carrier"), 1u8)
}

fn extract_oui(mac_address: &str) -> Option<String> {
    let mac_parts: Vec<&str> = mac_address.split(':').collect();

    (mac_parts.len() == 6
        && mac_parts[0..3]
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit())))
    .then(|| mac_parts[0..3].join(":").to_uppercase())
}

pub fn get_active_wireless_ap_ouis() -> Option<Vec<(String, String)>> {
    // https://www.man7.org/linux/man-pages/man5/proc_net.5.html
    // eg:
    // $ cat /proc/net/arp
    // IP address       HW type     Flags       HW address            Mask     Device
    // 192.168.128.1    0x1         0x2         XX:XX:XX:XX:XX:XX     *        wlp0s20f3
    // 172.18.0.2       0x1         0x2         XX:XX:XX:XX:XX:XX     *        br-7bd4974c1fae
    let arp_content = read_file_content(Path::new(PROC_NET_ARP))?;
    let mut ap_ouis = Vec::new();

    for line in arp_content.lines().skip(1) {
        if let Some((device, oui)) = parse_arp_entry(line) {
            // Go off and read /sys/class/net/$DEVICE
            if is_wireless_adapter(&Path::new(SYS_CLASS_NET).join(&device)) {
                ap_ouis.push((device, oui));
            }
        }
    }

    Some(ap_ouis)
}

fn parse_arp_entry(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.split_whitespace().collect();

    match (parts.get(3), parts.get(5)) {
        (Some(hw_address), Some(device)) => {
            extract_oui(hw_address).map(|oui| (device.to_string(), oui))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::{assert_json_snapshot, with_settings};
    use std::collections::HashMap;
    use std::fs;
    use std::io;
    use tempfile::TempDir;

    fn create_mock_adapter(
        base_path: &Path,
        name: &str,
        mac_address: &str,
        is_wireless: bool,
        is_active: bool,
    ) -> io::Result<()> {
        let adapter_path = base_path.join(name);
        fs::create_dir_all(&adapter_path)?;

        fs::write(adapter_path.join("type"), "1")?;
        fs::write(adapter_path.join("address"), mac_address)?;

        if is_wireless {
            fs::create_dir_all(adapter_path.join("wireless"))?;
        }

        if is_active {
            fs::write(adapter_path.join("operstate"), "up")?;
            fs::write(adapter_path.join("carrier"), "1")?;
        } else {
            fs::write(adapter_path.join("operstate"), "down")?;
            fs::write(adapter_path.join("carrier"), "0")?;
        }

        Ok(())
    }

    #[test]
    fn test_extract_oui() {
        assert_eq!(
            extract_oui("aa:bb:cc:dd:ee:ff"),
            Some("AA:BB:CC".to_string())
        );
        assert_eq!(
            extract_oui("01:23:45:67:89:ab"),
            Some("01:23:45".to_string())
        );
        assert_eq!(extract_oui("invalid_mac"), None);
        assert_eq!(extract_oui("aa:bb:cc:dd:ee"), None);
        assert_eq!(extract_oui("aa:bb:cc:dd:ee:ff:gg"), None);
        assert_eq!(extract_oui("aa:bb:gg:dd:ee:ff"), None);
    }

    fn get_wireless_local_ouis_with_path(sys_net_path: &Path) -> Option<Vec<(String, String)>> {
        let mut ouis = Vec::new();

        let entries = fs::read_dir(sys_net_path).ok()?;

        for entry in entries.flatten() {
            let adapter_name = entry.file_name().to_string_lossy().to_string();
            let adapter_path = sys_net_path.join(&adapter_name);

            if is_wireless_adapter(&adapter_path) && is_adapter_active(&adapter_path) {
                if let Some(oui) = read_file_content(&adapter_path.join("address"))
                    .and_then(|mac| extract_oui(&mac))
                {
                    ouis.push((adapter_name, oui));
                }
            }
        }

        Some(ouis)
    }

    #[test]
    fn test_get_wireless_local_ouis() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "wlp3s0", "aa:bb:cc:dd:ee:ff", true, true)?;
        create_mock_adapter(mock_sys_path, "eth0", "11:22:33:44:55:66", false, true)?;
        create_mock_adapter(mock_sys_path, "wlp4s0", "77:88:99:aa:bb:cc", true, false)?;
        create_mock_adapter(mock_sys_path, "wlan0", "12:34:56:78:90:ab", true, true)?;

        let ouis = get_wireless_local_ouis_with_path(mock_sys_path).unwrap();

        assert_eq!(ouis.len(), 2);
        let oui_map: std::collections::HashMap<String, String> = ouis.into_iter().collect();
        assert_eq!(oui_map.get("wlp3s0"), Some(&"AA:BB:CC".to_string()));
        assert_eq!(oui_map.get("wlan0"), Some(&"12:34:56".to_string()));

        Ok(())
    }

    #[test]
    fn test_non_wireless_adapter_filtered() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "eth0", "11:22:33:44:55:66", false, true)?;

        let ouis = get_wireless_local_ouis_with_path(mock_sys_path).unwrap();
        assert_eq!(ouis.len(), 0);

        Ok(())
    }

    #[test]
    fn test_inactive_adapter_filtered() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "wlan0", "aa:bb:cc:dd:ee:ff", true, false)?;

        let ouis = get_wireless_local_ouis_with_path(mock_sys_path).unwrap();
        assert_eq!(ouis.len(), 0);

        Ok(())
    }

    #[test]
    fn test_empty_directory() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        let ouis = get_wireless_local_ouis_with_path(mock_sys_path).unwrap();
        assert_eq!(ouis.len(), 0);

        Ok(())
    }

    #[test]
    fn test_malformed_adapter_directory() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        let adapter_path = mock_sys_path.join("broken_adapter");
        fs::create_dir_all(adapter_path)?;

        let ouis = get_wireless_local_ouis_with_path(mock_sys_path).unwrap();
        assert_eq!(ouis.len(), 0);

        Ok(())
    }

    fn create_mock_arp_content() -> String {
        "IP address       HW type     Flags       HW address            Mask     Device\n172.18.0.2       0x1         0x2         ea:6f:b3:e4:19:15     *        wlp0s20f3\n192.168.128.1    0x1         0x2         bc:db:09:c2:4c:95     *        eth0\n10.0.0.1         0x1         0x2         aa:bb:cc:dd:ee:ff     *        wlan0".to_string()
    }

    fn get_wireless_ap_ouis_with_content(
        arp_content: &str,
        sys_net_path: &Path,
    ) -> Option<Vec<(String, String)>> {
        let mut ap_ouis = Vec::new();

        for line in arp_content.lines().skip(1) {
            if let Some((device, oui)) = parse_arp_entry(line) {
                let sys_device_path = sys_net_path.join(&device);
                if is_wireless_adapter(&sys_device_path) {
                    ap_ouis.push((device, oui));
                }
            }
        }

        Some(ap_ouis)
    }

    #[test]
    fn test_parse_arp_entry() {
        assert_eq!(
            parse_arp_entry(
                "172.18.0.2       0x1         0x2         ea:6f:b3:e4:19:15     *        wlp0s20f3"
            ),
            Some(("wlp0s20f3".to_string(), "EA:6F:B3".to_string()))
        );

        assert_eq!(
            parse_arp_entry(
                "192.168.1.1    0x1         0x2         bc:db:09:c2:4c:95     *        eth0"
            ),
            Some(("eth0".to_string(), "BC:DB:09".to_string()))
        );

        assert_eq!(parse_arp_entry("invalid line"), None);
        assert_eq!(
            parse_arp_entry(
                "IP address       HW type     Flags       HW address            Mask     Device"
            ),
            None
        );
    }

    #[test]
    fn test_get_wireless_ap_ouis() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "wlp0s20f3", "11:22:33:44:55:66", true, true)?;
        create_mock_adapter(mock_sys_path, "eth0", "77:88:99:aa:bb:cc", false, true)?;
        create_mock_adapter(mock_sys_path, "wlan0", "aa:bb:cc:dd:ee:ff", true, true)?;

        let arp_content = create_mock_arp_content();
        let ap_ouis = get_wireless_ap_ouis_with_content(&arp_content, mock_sys_path).unwrap();

        assert_eq!(ap_ouis.len(), 2);
        let oui_map: std::collections::HashMap<String, String> = ap_ouis.into_iter().collect();
        assert_eq!(oui_map.get("wlp0s20f3"), Some(&"EA:6F:B3".to_string()));
        assert_eq!(oui_map.get("wlan0"), Some(&"AA:BB:CC".to_string()));

        Ok(())
    }

    #[test]
    fn test_non_wireless_device_filtered_from_arp() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "eth0", "77:88:99:aa:bb:cc", false, true)?;

        let arp_content = create_mock_arp_content();
        let ap_ouis = get_wireless_ap_ouis_with_content(&arp_content, mock_sys_path).unwrap();

        assert_eq!(ap_ouis.len(), 0);

        Ok(())
    }

    #[test]
    fn test_empty_arp_content() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        let arp_content =
            "IP address       HW type     Flags       HW address            Mask     Device";
        let ap_ouis = get_wireless_ap_ouis_with_content(arp_content, mock_sys_path).unwrap();

        assert_eq!(ap_ouis.len(), 0);

        Ok(())
    }

    #[test]
    fn test_malformed_arp_entries() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "wlan0", "aa:bb:cc:dd:ee:ff", true, true)?;

        let arp_content = "IP address       HW type     Flags       HW address            Mask     Device\ninvalid line\nincomplete";
        let ap_ouis = get_wireless_ap_ouis_with_content(arp_content, mock_sys_path).unwrap();

        assert_eq!(ap_ouis.len(), 0);

        Ok(())
    }

    #[test]
    fn test_get_wireless_local_ouis_json_snapshot() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "wlp3s0", "aa:bb:cc:dd:ee:ff", true, true)?;
        create_mock_adapter(mock_sys_path, "eth0", "11:22:33:44:55:66", false, true)?;
        create_mock_adapter(mock_sys_path, "wlp4s0", "77:88:99:aa:bb:cc", true, false)?;
        create_mock_adapter(mock_sys_path, "wlan0", "12:34:56:78:90:ab", true, true)?;

        let ouis: HashMap<String, String> = get_wireless_local_ouis_with_path(mock_sys_path)
            .unwrap()
            .into_iter()
            .collect();

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(ouis);
        });

        Ok(())
    }

    #[test]
    fn test_get_wireless_ap_ouis_json_snapshot() -> io::Result<()> {
        let temp_dir = TempDir::new()?;
        let mock_sys_path = temp_dir.path();

        create_mock_adapter(mock_sys_path, "wlp0s20f3", "11:22:33:44:55:66", true, true)?;
        create_mock_adapter(mock_sys_path, "eth0", "77:88:99:aa:bb:cc", false, true)?;
        create_mock_adapter(mock_sys_path, "wlan0", "aa:bb:cc:dd:ee:ff", true, true)?;

        let arp_content = create_mock_arp_content();
        let ap_ouis: HashMap<String, String> =
            get_wireless_ap_ouis_with_content(&arp_content, mock_sys_path)
                .unwrap()
                .into_iter()
                .collect();

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(ap_ouis);
        });

        Ok(())
    }
}
