use core::net::Ipv4Addr;
use core::str;
use std::{
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::bail;
use esp_idf_svc::{
    nvs::{EspNvs, EspNvsPartition, NvsDefault},
    wifi::{
        AccessPointConfiguration, AsyncWifi, AuthMethod, ClientConfiguration,
        Configuration as WifiConfiguration, EspWifi,
    },
};
use log::{info, warn};

use crate::led::set_led;
use crate::LedType;

async fn wifi_client(
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
        info!("Wifi password is empty");
    }

    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration::default()))?;

    info!("Starting wifi...");

    wifi.start().await?;

    info!("Scanning...");

    let ap_infos = wifi.scan().await?;

    let ours = ap_infos.into_iter().find(|a| a.ssid == ssid);

    let channel = if let Some(ours) = ours {
        info!(
            "Found configured access point {} on channel {}",
            ssid, ours.channel
        );
        Some(ours.channel)
    } else {
        info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            ssid
        );
        None
    };

    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration {
        ssid: ssid
            .try_into()
            .expect("Could not parse the given SSID into WiFi config"),
        password: pass
            .try_into()
            .expect("Could not parse the given password into WiFi config"),
        channel,
        auth_method,
        ..Default::default()
    }))?;

    info!("Connecting wifi...");

    wifi.connect().await?;

    info!("Waiting for DHCP lease...");

    wifi.wait_netif_up().await?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    info!("Wifi DHCP info: {:?}", ip_info);

    Ok(())
}

#[derive(Debug)]
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
    let nvs = match EspNvs::new(nvs, "wifi", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            warn!("NVS read error, starting hotspot");
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
        info!("SSID and/or Password empty");
        let ip = wifi_hotspot(wifi).await?;
        set_led(ws2812, 255, 0, 255);
        let mut w_status = wifi_status.lock().unwrap();
        *w_status = WifiEnum::HotSpot;
        return Ok((WifiEnum::HotSpot, Some(ip)));
    }

    let wifi_client_res = wifi_client(wifi_ssid.unwrap(), wifi_password.unwrap(), wifi).await;
    if wifi_client_res.is_err() {
        warn!("Wifi connection failed, falling back to hotspot");
        let ip = wifi_hotspot(wifi).await?;
        set_led(ws2812, 255, 0, 255);
        let mut w_status = wifi_status.lock().unwrap();
        *w_status = WifiEnum::HotSpot;
        return Ok((WifiEnum::HotSpot, Some(ip)));
    }
    set_led(ws2812, 0, 255, 0);
    let mut w_status = wifi_status.lock().unwrap();
    *w_status = WifiEnum::Connected;
    Ok((WifiEnum::Connected, None))
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
    wifi.set_configuration(&WifiConfiguration::AccessPoint(AccessPointConfiguration {
        ssid: heapless::String::from_str("Td-Free").unwrap(),
        auth_method: AuthMethod::None,
        channel: 11,
        ssid_hidden: false,
        password: "".try_into().unwrap(),
        ..Default::default()
    }))?;

    info!("Starting wifi...");

    wifi.start().await?;

    info!("Waiting for DHCP lease...");

    wifi.wait_netif_up().await?;
    let ipv4_address = wifi.wifi().ap_netif().get_ip_info().unwrap();

    Ok(ipv4_address.ip)
}
