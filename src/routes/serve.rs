pub fn serve_wifi_setup_page(current_ssid: &str, error: &str) -> String {
    format!(
        include_str!("static/wifi_setup.html"),
        ssid = current_ssid,
        error = error
    )
}

pub fn serve_algo_setup_page(
    b_val: f32,
    m_val: f32,
    threshold_val: f32,
    spoolman_val: &str,
    spoolman_field_name: &str,
) -> String {
    format!(
        include_str!("static/settings.html"),
        b_val = b_val,
        m_val = m_val,
        threshold_val = threshold_val,
        spoolman_val = spoolman_val,
        spoolman_field_name = spoolman_field_name
    )
}
