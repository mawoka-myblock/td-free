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
    wifi_status: Arc<Mutex<WifiEnum>>,
) -> anyhow::Result<WifiEnum> {
    let nvs = match EspNvs::new(nvs, "wifi", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            warn!("NVS read error, starting hotspot");
            wifi_hotspot(wifi).await?;
            let mut w_status = wifi_status.lock().unwrap();
            *w_status = WifiEnum::HotSpot;
            return Ok(WifiEnum::HotSpot);
        }
    };
    let mut wifi_ssid_buffer = vec![0; 256];
    let wifi_ssid = nvs.get_str("ssid", &mut wifi_ssid_buffer).unwrap();
    let mut wifi_password_buffer = vec![0; 256];
    let wifi_password = nvs.get_str("pw", &mut wifi_password_buffer).unwrap();
    if wifi_password.is_none() || wifi_ssid.is_none() {
        info!("SSID and/or Password empty");
        wifi_hotspot(wifi).await?;
        let mut w_status = wifi_status.lock().unwrap();
        *w_status = WifiEnum::HotSpot;
        return Ok(WifiEnum::HotSpot);
    }

    let wifi_client_res = wifi_client(wifi_ssid.unwrap(), wifi_password.unwrap(), wifi).await;
    if wifi_client_res.is_err() {
        warn!("Wifi connection failed, falling back to hotspot");
        wifi_hotspot(wifi).await?;
        let mut w_status = wifi_status.lock().unwrap();
        *w_status = WifiEnum::HotSpot;
        return Ok(WifiEnum::HotSpot);
    }
    let mut w_status = wifi_status.lock().unwrap();
    *w_status = WifiEnum::Connected;
    Ok(WifiEnum::Connected)
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

async fn wifi_hotspot(wifi: &mut AsyncWifi<EspWifi<'static>>) -> anyhow::Result<()> {
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

    Ok(())
}

/*
fn handle_response(
    req: esp_idf_svc::http::client::Request<&mut EspHttpConnection>,
) -> (u16, bool, Option<String>) {
    let response = req.submit().unwrap();
    let status = response.status();
    match status {
        200..=299 => {
            // 4. if the status is OK, read response data chunk by chunk into a buffer and print it until done
            //
            // NB. see http_client.rs for an explanation of the offset mechanism for handling chunks that are
            // split in the middle of valid UTF-8 sequences. This case is encountered a lot with the given
            // example URL.
            let mut buf = [0_u8; 256];
            let mut offset = 0;
            let mut resp_text = String::new();
            let mut reader = response;
            loop {
                if let Ok(size) = Read::read(&mut reader, &mut buf[offset..]) {
                    if size == 0 {
                        break;
                    }
                    // 5. try converting the bytes into a Rust (UTF-8) string and print it
                    let size_plus_offset = size + offset;
                    match str::from_utf8(&buf[..size_plus_offset]) {
                        Ok(text) => {
                            resp_text.push_str(text);
                            offset = 0;
                        }
                        Err(error) => {
                            let valid_up_to = error.valid_up_to();
                            unsafe {
                                resp_text.push_str(str::from_utf8_unchecked(&buf[..valid_up_to]));
                            }
                            buf.copy_within(valid_up_to.., 0);
                            offset = size_plus_offset - valid_up_to;
                        }
                    }
                }
            }
            (status, true, Some(resp_text).filter(|x| !x.is_empty()))
        }
        _ => (status, false, None),
    }
}

pub fn get(url: impl AsRef<str>) -> Result<(u16, bool, Option<String>)> {
    // 1. Create a new EspHttpClient. (Check documentation)
    // ANCHOR: connection
    let connection = EspHttpConnection::new(&HttpConfiguration {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        ..Default::default()
    })?;
    // ANCHOR_END: connection
    let mut client = Client::wrap(connection);

    // 2. Open a GET request to `url`
    let headers = [("accept", "text/plain")];
    let request = client.request(Method::Get, url.as_ref(), &headers)?;

    Ok(handle_response(request))
}

pub fn post(
    url: impl AsRef<str>,
    body: &[u8],
    content_type: &str,
) -> Result<(u16, bool, Option<String>)> {
    let content_length = body.len().to_string();
    let connection = EspHttpConnection::new(&HttpConfiguration {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        raw_request_body: true,
        ..Default::default()
    })?;
    let mut client = Client::wrap(connection);
    let headers = [
        ("Content-Type", content_type),
        ("Content-Length", &content_length),
    ];
    let mut request = client.request(Method::Post, url.as_ref(), &headers)?;
    request.write_all(body);
    request.flush().unwrap();
    Ok(handle_response(request))
}
 */
