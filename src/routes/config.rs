use std::{borrow::Cow, collections::HashMap};

use edge_http::io::Error as EdgeError;
use edge_http::io::server::Connection;
use embedded_io_async::{Read, Write};
use esp_idf_svc::hal::reset;
use log::error;
use url::Url;

use crate::{
    WsHandler, WsHandlerError,
    helpers::nvs::{
        get_saved_algorithm_variables, read_spoolman_data, save_algorithm_variables,
        save_spoolman_data,
    },
    routes::serve::{serve_algo_setup_page, serve_wifi_setup_page},
    wifi,
};

impl WsHandler<'_> {
    pub async fn wifi_route<T, const N: usize>(
        &self,
        path: &str,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        let url = Url::parse(&format!("http://google.com{path}")).unwrap();
        let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let ssid = url_params.get("ssid");
        let password = url_params.get("password");
        if ssid.is_none() && password.is_none() {
            let saved_ssid =
                wifi::get_wifi_ssid(self.nvs.clone().as_ref().clone()).unwrap_or_default();
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(serve_wifi_setup_page(&saved_ssid, "").as_ref())
                .await?;
            return Ok(());
        }
        if ssid.is_none() {
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(serve_wifi_setup_page("", "SSID is not set").as_ref())
                .await?;
            return Ok(());
        }
        if password.is_none() {
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(serve_wifi_setup_page("", "SSID is not set").as_ref())
                .await?;
            return Ok(());
        }
        match wifi::save_wifi_creds(
            ssid.unwrap(),
            password.unwrap(),
            self.nvs.clone().as_ref().clone(),
        ) {
            Ok(_) => {
                conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                    .await?;
                conn.write_all(
                    serve_wifi_setup_page(
                        ssid.unwrap_or(&String::new()),
                        "Saved successfully, resetting now",
                    )
                    .as_ref(),
                )
                .await?;
                embassy_time::Timer::after_millis(50).await;
                reset::restart();
            }
            Err(e) => {
                conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                    .await?;
                conn.write_all(
                    serve_wifi_setup_page(
                        ssid.unwrap_or(&String::new()),
                        "COULD NOT SAVE WIFI CREDENTIALS, resetting now",
                    )
                    .as_ref(),
                )
                .await?;
                error!("{e:?}");
                embassy_time::Timer::after_millis(50).await;
                reset::restart();
            }
        };
    }

    pub async fn algorithm_route<T, const N: usize>(
        &self,
        path: &str,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        let url = Url::parse(&format!("http://google.com{path}")).unwrap();
        let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let m_value = url_params.get("m");
        let b_value = url_params.get("b");
        let threshold_value = url_params.get("threshold");
        let spoolman_value = url_params.get("spoolman_url");
        let spoolman_field_name = url_params.get("spoolman_field_name");
        if m_value.is_none()
            && b_value.is_none()
            && threshold_value.is_none()
            && spoolman_value.is_none()
        {
            let saved_algorithm = get_saved_algorithm_variables(self.nvs.as_ref().clone());
            let saved_spoolman = read_spoolman_data(self.nvs.as_ref().clone());
            let spoolman_url = match saved_spoolman.0 {
                Some(d) => d,
                None => "".to_string(),
            };
            let spoolman_field_name = match saved_spoolman.1 {
                Some(d) => d,
                None => "td".to_string(),
            };
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(
                serve_algo_setup_page(
                    saved_algorithm.b,
                    saved_algorithm.m,
                    saved_algorithm.threshold,
                    &spoolman_url,
                    &spoolman_field_name,
                )
                .as_ref(),
            )
            .await?;
            return Ok(());
        }
        let mod_b_value = b_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("0.0".to_string()));
        let mod_m_value = m_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("1.0".to_string()));
        let mod_threshold_value = threshold_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("0.5".to_string()));
        let mod_spoolman_value = spoolman_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("".to_string()));
        let mod_spoolman_field_name = spoolman_field_name
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("".to_string()));
        let save_spoolman_res = save_spoolman_data(
            &mod_spoolman_value,
            &mod_spoolman_field_name,
            self.nvs.as_ref().clone(),
        );
        if save_spoolman_res.is_err() {
            error!("{:?}", save_spoolman_res.err().unwrap());
            embassy_time::Timer::after_millis(50).await;
            reset::restart();
        }
        match save_algorithm_variables(
            &mod_b_value,
            &mod_m_value,
            &mod_threshold_value,
            self.nvs.as_ref().clone(),
        ) {
            Ok(_) => {
                conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                    .await?;
                conn.write_all(
                    serve_algo_setup_page(
                        mod_b_value.parse::<f32>().unwrap_or(0.0),
                        mod_m_value.parse::<f32>().unwrap_or(1.0),
                        mod_threshold_value.parse::<f32>().unwrap_or(0.5),
                        &mod_spoolman_value,
                        &mod_spoolman_field_name,
                    )
                    .as_ref(),
                )
                .await?;
                #[allow(clippy::needless_return)]
                return Ok(());
            }
            Err(e) => {
                error!("{e:?}");
                embassy_time::Timer::after_millis(50).await;
                reset::restart();
            }
        };
        // Ok(())
    }
}
