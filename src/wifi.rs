use core::net::Ipv4Addr;
use core::str;
use std::{
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::bail;
use esp_idf_svc::{
    nvs::{EspNvs, EspNvsPartition, NvsDefault},
    wifi::{
        AccessPointConfiguration, AsyncWifi, AuthMethod, ClientConfiguration,
        Configuration as WifiConfiguration, EspWifi,
    },
};
use log::{info, warn, error, debug};

use crate::led::set_led;
use crate::LedType;

const MAX_CONNECTION_ATTEMPTS: u8 = 3;
const CONNECTION_TIMEOUT_MS: u64 = 10000; // 10 seconds
const SCAN_RETRY_COUNT: u8 = 3;
const MIN_SIGNAL_STRENGTH: i8 = -80; // dBm - minimum acceptable signal strength

async fn wifi_client_with_retries(
    ssid: &str,
    pass: &str,
    wifi: &mut AsyncWifi<EspWifi<'static>>,
) -> anyhow::Result<()> {
    for attempt in 1..=MAX_CONNECTION_ATTEMPTS {
        info!("WiFi connection attempt {} of {}", attempt, MAX_CONNECTION_ATTEMPTS);

        match wifi_client_single_attempt(ssid, pass, wifi).await {
            Ok(_) => {
                info!("WiFi connected successfully on attempt {}", attempt);
                return Ok(());
            }
            Err(e) => {
                error!("WiFi connection attempt {} failed: {:?}", attempt, e);

                if attempt < MAX_CONNECTION_ATTEMPTS {
                    // Stop and restart WiFi between attempts to reset state
                    info!("Resetting WiFi for next attempt...");
                    let _ = wifi.stop().await; // Ignore errors when stopping
                    embassy_time::Timer::after_millis(2000).await; // Wait 2 seconds between attempts
                } else {
                    error!("All WiFi connection attempts failed");
                    return Err(e);
                }
            }
        }
    }

    bail!("Failed to connect after {} attempts", MAX_CONNECTION_ATTEMPTS)
}

async fn wifi_client_single_attempt(
    ssid: &str,
    pass: &str,
    wifi: &mut AsyncWifi<EspWifi<'static>>,
) -> anyhow::Result<()> {
    let mut auth_method = AuthMethod::WPA2Personal;
    if ssid.is_empty() {
        bail!("Missing WiFi name")
    }
    if pass.is_empty() {
        auth_method = AuthMethod::None;
        info!("Wifi password is empty, using hotspot");
    }

    // Set initial client configuration
    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration::default()))?;

    info!("Starting wifi...");
    wifi.start().await?;

    // Add delay after start
    embassy_time::Timer::after_millis(1000).await;

    info!("Scanning for networks...");

    // Retry scanning if it fails
    let ap_infos = scan_with_retries(wifi).await?;

    let ours = ap_infos.iter().find(|a| a.ssid == ssid);

    let (channel, signal_strength) = if let Some(ours) = ours {
        info!(
            "Found configured access point {} on channel {} with signal strength {} dBm",
            ssid, ours.channel, ours.signal_strength
        );

        // Check signal strength
        if ours.signal_strength < MIN_SIGNAL_STRENGTH {
            warn!(
                "Signal strength {} dBm is below minimum {} dBm, but attempting connection anyway",
                ours.signal_strength, MIN_SIGNAL_STRENGTH
            );
        }

        // Determine the best auth method based on scan results
        let detected_auth = match ours.auth_method {
            Some(auth) => {
                info!("Detected auth method: {:?}", auth);
                auth
            }
            None => {
                info!("No auth method detected, using configured method");
                auth_method
            }
        };

        (Some(ours.channel), ours.signal_strength)
    } else {
        warn!(
            "Configured access point {} not found during scanning. Available networks:",
            ssid
        );

        for ap in &ap_infos {
            debug!("  - {} (channel {}, {} dBm)", ap.ssid, ap.channel, ap.signal_strength);
        }

        // Still attempt connection with unknown channel
        (None, 0)
    };

    // Configure WiFi with discovered parameters
    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration {
        ssid: ssid
            .try_into()
            .map_err(|e| anyhow::anyhow!("Could not parse SSID '{}': {:?}", ssid, e))?,
        password: pass
            .try_into()
            .map_err(|e| anyhow::anyhow!("Could not parse password: {:?}", e))?,
        channel,
        auth_method,
        ..Default::default()
    }))?;

    info!("Connecting to WiFi network '{}'...", ssid);

    // Connect with timeout
    let connect_result = embassy_futures::select::select(
        wifi.connect(),
        embassy_time::Timer::after_millis(CONNECTION_TIMEOUT_MS)
    ).await;

    match connect_result {
        embassy_futures::select::Either::First(Ok(_)) => {
            info!("WiFi connect command successful");
        }
        embassy_futures::select::Either::First(Err(e)) => {
            bail!("WiFi connect failed: {:?}", e);
        }
        embassy_futures::select::Either::Second(_) => {
            bail!("WiFi connect timed out after {}ms", CONNECTION_TIMEOUT_MS);
        }
    }

    info!("Waiting for network interface to come up...");

    // Wait for network interface with timeout
    let netif_result = embassy_futures::select::select(
        wifi.wait_netif_up(),
        embassy_time::Timer::after_millis(CONNECTION_TIMEOUT_MS)
    ).await;

    match netif_result {
        embassy_futures::select::Either::First(Ok(_)) => {
            info!("Network interface is up");
        }
        embassy_futures::select::Either::First(Err(e)) => {
            bail!("Network interface failed to come up: {:?}", e);
        }
        embassy_futures::select::Either::Second(_) => {
            bail!("Network interface timed out after {}ms", CONNECTION_TIMEOUT_MS);
        }
    }

    // Verify we got an IP address
    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    if ip_info.ip == Ipv4Addr::new(0, 0, 0, 0) {
        bail!("Failed to obtain IP address from DHCP");
    }

    info!("WiFi DHCP info: {:?}", ip_info);
    info!("Successfully connected to WiFi network '{}'", ssid);

    Ok(())
}

async fn scan_with_retries(wifi: &mut AsyncWifi<EspWifi<'static>>) -> anyhow::Result<Vec<esp_idf_svc::wifi::AccessPointInfo>> {
    for attempt in 1..=SCAN_RETRY_COUNT {
        debug!("WiFi scan attempt {} of {}", attempt, SCAN_RETRY_COUNT);

        match wifi.scan().await {
            Ok(ap_infos) => {
                if ap_infos.is_empty() {
                    warn!("Scan attempt {} returned no networks", attempt);
                    if attempt < SCAN_RETRY_COUNT {
                        embassy_time::Timer::after_millis(1000).await;
                        continue;
                    }
                } else {
                    info!("Scan successful, found {} networks", ap_infos.len());
                    return Ok(ap_infos);
                }
            }
            Err(e) => {
                warn!("Scan attempt {} failed: {:?}", attempt, e);
                if attempt < SCAN_RETRY_COUNT {
                    embassy_time::Timer::after_millis(1000).await;
                    continue;
                }
            }
        }
    }

    bail!("Failed to scan networks after {} attempts", SCAN_RETRY_COUNT)
}

#[derive(Debug, PartialEq)]
pub enum WifiEnum {
    HotSpot,
    Connected,
    Working,
}

pub async fn wifi_setup(
    wifi: &mut AsyncWifi<EspWifi<'static>>,
    nvs: EspNvsPartition<NvsDefault>,
    ws2812: Arc<Mutex<LedType<'_>>>,
    wifi_status: Arc<Mutex<WifiEnum>>,
) -> anyhow::Result<(WifiEnum, Option<Ipv4Addr>)> {
    // Set status to working while attempting connection
    {
        let mut w_status = wifi_status.lock().unwrap();
        *w_status = WifiEnum::Working;
    }
    set_led(ws2812.clone(), 255, 255, 0); // Yellow for working

    let nvs = match EspNvs::new(nvs, "wifi", true) {
        Ok(nvs) => nvs,
        Err(e) => {
            error!("NVS read error: {:?}, starting hotspot", e);
            let ip = wifi_hotspot(wifi).await?;
            set_led(ws2812, 255, 0, 255);
            let mut w_status = wifi_status.lock().unwrap();
            *w_status = WifiEnum::HotSpot;
            return Ok((WifiEnum::HotSpot, Some(ip)));
        }
    };

    let mut wifi_ssid_buffer = vec![0; 256];
    let wifi_ssid = nvs.get_str("ssid", &mut wifi_ssid_buffer).unwrap();
    let mut wifi_password_buffer = vec![0; 256];
    let wifi_password = nvs.get_str("pw", &mut wifi_password_buffer).unwrap();

    if wifi_password.is_none() || wifi_ssid.is_none() {
        info!("SSID and/or Password not configured, starting hotspot");
        let ip = wifi_hotspot(wifi).await?;
        set_led(ws2812, 255, 0, 255);
        let mut w_status = wifi_status.lock().unwrap();
        *w_status = WifiEnum::HotSpot;
        return Ok((WifiEnum::HotSpot, Some(ip)));
    }

    let ssid = wifi_ssid.unwrap();
    let password = wifi_password.unwrap();

    info!("Attempting to connect to WiFi network: '{}'", ssid);

    let wifi_client_res = wifi_client_with_retries(ssid, password, wifi).await;

    match wifi_client_res {
        Ok(_) => {
            info!("Successfully connected to WiFi network '{}'", ssid);
            set_led(ws2812, 0, 255, 0); // Green for connected
            let mut w_status = wifi_status.lock().unwrap();
            *w_status = WifiEnum::Connected;
            Ok((WifiEnum::Connected, None))
        }
        Err(e) => {
            error!("WiFi client connection failed after all attempts: {:?}", e);
            warn!("Falling back to hotspot mode");

            // Stop WiFi before switching to hotspot
            let _ = wifi.stop().await;
            embassy_time::Timer::after_millis(1000).await;

            let ip = wifi_hotspot(wifi).await?;
            set_led(ws2812, 255, 0, 255); // Magenta for hotspot
            let mut w_status = wifi_status.lock().unwrap();
            *w_status = WifiEnum::HotSpot;
            Ok((WifiEnum::HotSpot, Some(ip)))
        }
    }
}

pub fn save_wifi_creds(
    ssid: &str,
    password: &str,
    nvs: EspNvsPartition<NvsDefault>,
) -> anyhow::Result<()> {
    let mut nvs = match EspNvs::new(nvs, "wifi", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            bail!("NVS failed");
        }
    };
    nvs.set_str("ssid", ssid)?;
    nvs.set_str("pw", password)?;
    Ok(())
}

pub fn get_wifi_ssid(nvs: EspNvsPartition<NvsDefault>) -> Option<String> {
    let nvs = match EspNvs::new(nvs, "wifi", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            warn!("NVS init failed");
            return None;
        }
    };
    let mut wifi_ssid_buffer = vec![0; 256];
    nvs.get_str("ssid", &mut wifi_ssid_buffer)
        .unwrap()
        .map(|s| s.to_string())
}

async fn wifi_hotspot(wifi: &mut AsyncWifi<EspWifi<'static>>) -> anyhow::Result<Ipv4Addr> {
    info!("Starting WiFi hotspot...");

    wifi.set_configuration(&WifiConfiguration::AccessPoint(AccessPointConfiguration {
        ssid: heapless::String::from_str("Td-Free").unwrap(),
        auth_method: AuthMethod::None,
        channel: 11,
        ssid_hidden: false,
        password: "".try_into().unwrap(),
        max_connections: 4, // Limit concurrent connections
        ..Default::default()
    }))?;

    info!("Starting WiFi in hotspot mode...");
    wifi.start().await?;

    info!("Waiting for hotspot interface to come up...");

    // Wait for interface with timeout
    let netif_result = embassy_futures::select::select(
        wifi.wait_netif_up(),
        embassy_time::Timer::after_millis(10000) // 10 second timeout for hotspot
    ).await;

    match netif_result {
        embassy_futures::select::Either::First(Ok(_)) => {
            let ipv4_address = wifi.wifi().ap_netif().get_ip_info()?;
            info!("WiFi hotspot started successfully at IP: {}", ipv4_address.ip);
            Ok(ipv4_address.ip)
        }
        embassy_futures::select::Either::First(Err(e)) => {
            bail!("Hotspot interface failed to come up: {:?}", e);
        }
        embassy_futures::select::Either::Second(_) => {
            bail!("Hotspot interface timed out");
        }
    }
}
