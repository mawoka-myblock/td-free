use defmt::{Debug2Format, error, info, warn};
use embassy_net::{Runner, Stack};
use embassy_time::Timer;
use esp_radio::wifi::{AccessPointStationEventInfo, Interface, WifiController};

#[embassy_executor::task]
pub async fn run_dhcp_server(stack: Stack<'static>) {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let ip = Ipv4Addr::new(10, 10, 10, 1);

    let mut buf = [0u8; 1500];

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();
    let dns = [ip];
    let mut server_options = ServerOptions::new(ip, Some(&mut gw_buf));
    server_options.dns = &dns;
    server_options.captive_url = Some("http://10.10.10.1");

    loop {
        _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(ip),
            &server_options,
            &mut bound_socket,
            &mut buf,
        )
        .await
        .inspect_err(|e| warn!("DHCP server error: {:?}", Debug2Format(&e)));
        Timer::after_millis(500).await;
    }
}

#[embassy_executor::task]
pub async fn captive_dns(stack: Stack<'static>) {
    use core::net::{IpAddr, Ipv4Addr, SocketAddr};
    use core::time::Duration;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let mut tx_buf = [0; 1500];
    let mut rx_buf = [0; 1500];

    let buffers = UdpBuffers::<1, 256, 256, 1>::new();
    let unbound_socket = Udp::new(stack, &buffers);
    loop {
        match edge_captive::io::run(
            &unbound_socket,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 53),
            &mut tx_buf,
            &mut rx_buf,
            Ipv4Addr::new(10, 10, 10, 1),
            Duration::from_secs(99999),
        )
        .await
        {
            Ok(_) => (),
            Err(e) => error!("{:?}", Debug2Format(&e)),
        };
    }
}

#[embassy_executor::task]
pub async fn listen_for_connect_event_wifi_ap(controller: WifiController<'static>) {
    info!("start connection task");
    loop {
        let ev = controller
            .wait_for_access_point_connected_event_async()
            .await;
        match ev {
            Ok(AccessPointStationEventInfo::Connected(info)) => {
                info!("Station connected: {:?}", info);
            }
            Ok(AccessPointStationEventInfo::Disconnected(info)) => {
                info!("Station disconnected: {:?}", info);
            }
            _ => (),
        }
        Timer::after_millis(5000).await
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}
