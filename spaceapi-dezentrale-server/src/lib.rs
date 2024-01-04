#![allow(clippy::let_unit_value)]

pub mod config;
pub mod routes;
pub mod state;

#[macro_use]
extern crate rocket;

use crate::{
    config::{ApiKey, SpaceConfig},
    routes::{
        admin_ui_control, admin_ui_view, close_space, get_status_html, get_status_text, get_status_v14,
        index, keep_open, open_space, options_catch_all, Cors,
    },
    state::SpaceGuard,
};
use rocket::{Build, Rocket};
use std::time::{SystemTime, UNIX_EPOCH};

const SOFTWARE: &str = std::env!("CARGO_PKG_NAME");
const VERSION: &str = std::env!("CARGO_PKG_VERSION");

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Unix time")
        .as_secs()
}

pub async fn serve(config: SpaceConfig) -> Rocket<Build> {
    let space_guard = SpaceGuard::new(config.admin.keep_open_interval);
    space_guard.start_scheduler(config.admin.keep_open_interval).await;

    let mut routes = routes![
        admin_ui_control,
        admin_ui_view,
        get_status_v14,
        get_status_html,
        get_status_text,
        keep_open,
        index,
        options_catch_all
    ];

    if config.admin.enabled {
        routes.extend(routes![open_space, close_space]);
    }

    let rocket = rocket::build()
        .attach(Cors)
        // Add loaded template for spaceapi publishing
        .manage(config.publish)
        .manage(config.status_display)
        // Add Space state
        .manage(space_guard)
        .mount("/", routes);

    if config.admin.enabled {
        // Add a generated API-KEY for admin interface
        rocket.manage(config.admin.api_key.unwrap_or(ApiKey::generate()))
    } else {
        rocket
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        config::{AdminConfig, StatusDisplay, StatusDisplayTypes},
        routes::*,
    };
    use rocket::{
        http::{Header, Status},
        local::asynchronous::Client,
        tokio,
    };
    use std::time::Duration;

    pub(crate) fn sample_config(admin_enabled: bool) -> SpaceConfig {
        let admin = if admin_enabled {
            AdminConfig {
                api_key: Some("sesame-open".into()),
                enabled: true,
                ..AdminConfig::default()
            }
        } else {
            AdminConfig {
                api_key: None,
                enabled: false,
                ..AdminConfig::default()
            }
        };

        SpaceConfig {
            publish: spaceapi_dezentrale::StatusBuilder::v14("test")
                .logo("some_logo")
                .url("http://localhost")
                .contact(Default::default())
                .location(Default::default())
                .build()
                .unwrap(),
            status_display: StatusDisplayTypes {
                text: StatusDisplay {
                    open: "text open".to_string(),
                    closed: "text closed".to_string(),
                },
                html: StatusDisplay {
                    open: "html open".to_string(),
                    closed: "html closed".to_string(),
                },
            },
            admin,
        }
    }

    pub(crate) async fn tester(config: SpaceConfig) -> Client {
        let rocket = serve(config).await.ignite().await.expect("A server");
        Client::tracked(rocket).await.expect("A client")
    }

    #[tokio::test]
    async fn check_space_name() {
        let client = tester(sample_config(false)).await;
        let response = client.get(uri!(get_status_v14())).dispatch().await;
        assert_eq!(Status::Ok, response.status());

        let response: spaceapi_dezentrale::Status = response.into_json().await.unwrap();
        assert_eq!("test", response.space);
    }

    fn admin_routes() -> Vec<String> {
        vec![uri!(open_space()).to_string(), uri!(close_space()).to_string()]
    }

    #[tokio::test]
    async fn check_enabled_admin_api_not_authorized_without_api_key() {
        let client = tester(sample_config(true)).await;
        for route in admin_routes() {
            let response = client.post(route).dispatch().await;
            assert_eq!(Status::Unauthorized, response.status());
        }
    }

    #[tokio::test]
    async fn check_enabled_admin_api_not_authorized_with_invalid_api_key() {
        let client = tester(sample_config(true)).await;

        for route in admin_routes() {
            let response = client
                .post(route)
                .header(Header::new("X-API-KEY", "sesame"))
                .dispatch()
                .await;
            assert_eq!(Status::Unauthorized, response.status());
        }
    }

    #[tokio::test]
    async fn check_enabled_admin_api_authorized_with_valid_api_key() {
        let client = tester(sample_config(true)).await;

        for route in admin_routes() {
            let response = client
                .post(route)
                .header(Header::new("X-API-KEY", "sesame-open"))
                .dispatch()
                .await;
            assert_eq!(Status::Ok, response.status());
        }
    }

    #[tokio::test]
    async fn check_disabled_admin_api() {
        let client = tester(sample_config(false)).await;

        for route in admin_routes() {
            let response = client.post(route).dispatch().await;
            assert_eq!(Status::NotFound, response.status());
        }
    }

    #[tokio::test]
    async fn check_status_text() {
        let client = tester(sample_config(true)).await;

        let response = client.get(uri!(get_status_text())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("text closed", response.into_string().await.unwrap());

        let response = client
            .post(uri!(open_space()))
            .header(Header::new("X-API-KEY", "sesame-open"))
            .dispatch()
            .await;
        assert_eq!(Status::Ok, response.status());

        let response = client.get(uri!(get_status_text())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("text open", response.into_string().await.unwrap());
    }

    #[tokio::test]
    async fn check_status_html() {
        let client = tester(sample_config(true)).await;

        let response = client.get(uri!(get_status_html())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("html closed", response.into_string().await.unwrap());

        let response = client
            .post(uri!(open_space()))
            .header(Header::new("X-API-KEY", "sesame-open"))
            .dispatch()
            .await;
        assert_eq!(Status::Ok, response.status());

        let response = client.get(uri!(get_status_html())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("html open", response.into_string().await.unwrap());
    }

    #[tokio::test]
    async fn open_space_after_keep_open_request() {
        let client = tester(sample_config(true)).await;

        let response = client.get(uri!(get_status_html())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("html closed", response.into_string().await.unwrap());

        let response = client
            .post(uri!(keep_open()))
            .header(Header::new("X-API-KEY", "sesame-open"))
            .dispatch()
            .await;
        assert_eq!(Status::Ok, response.status());

        let response = client.get(uri!(get_status_html())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("html open", response.into_string().await.unwrap());
    }

    #[tokio::test]
    async fn close_space_after_keep_open_request() {
        let mut cfg = sample_config(true);
        cfg.admin.keep_open_interval = Duration::from_millis(5);
        cfg.admin.tick_interval = Duration::from_millis(3);
        let client = tester(cfg).await;

        let response = client.get(uri!(get_status_html())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("html closed", response.into_string().await.unwrap());

        let response = client
            .post(uri!(keep_open()))
            .header(Header::new("X-API-KEY", "sesame-open"))
            .dispatch()
            .await;
        assert_eq!(Status::Ok, response.status());

        tokio::time::sleep(Duration::from_millis(11)).await;

        let response = client.get(uri!(get_status_html())).dispatch().await;
        assert_eq!(Status::Ok, response.status());
        assert_eq!("html closed", response.into_string().await.unwrap());
    }
}
