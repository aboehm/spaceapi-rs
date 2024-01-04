use crate::{
    config::{ApiKey, StatusDisplayTypes},
    state::SpaceGuard,
};
use rocket::{
    fairing::{Fairing, Info, Kind},
    form::{Form, FromForm, FromFormField},
    http::{
        hyper::header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
        },
        ContentType, Header, Status,
    },
    outcome::Outcome,
    request::{self, FlashMessage, FromRequest, Request},
    response::{Flash, Redirect, Response},
    serde::json::Json,
    State,
};
use std::time::SystemTime;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ApiKey {
    type Error = &'static str;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        if let Some(api_key) = req.headers().get_one("X-API-Key") {
            if let Some(api_key_config) = req.rocket().state::<ApiKey>() {
                if api_key == api_key_config.0 {
                    return Outcome::Success(ApiKey(api_key.to_string()));
                }
            }
        }

        Outcome::Error((Status::Unauthorized, "Api key missing"))
    }
}

pub struct Cors;

/// Implementation for CORS
///
/// Inspired by
/// [Stackoverflow](https://stackoverflow.com/questions/62412361/how-to-set-up-cors-or-options-for-rocket-rs)
#[rocket::async_trait]
impl Fairing for Cors {
    fn info(&self) -> Info {
        Info {
            name: "CORS Settings",
            kind: Kind::Response,
        }
    }

    async fn on_response<'a>(&self, _request: &'a Request<'_>, response: &mut Response<'a>) {
        // Allow GET from all locations
        response.set_header(Header::new(ACCESS_CONTROL_ALLOW_HEADERS.as_str(), "X-API-Key"));
        response.set_header(Header::new(ACCESS_CONTROL_ALLOW_METHODS.as_str(), "POST,GET"));
        response.set_header(Header::new(ACCESS_CONTROL_ALLOW_ORIGIN.as_str(), "*"));
    }
}

#[post("/admin/publish/space-open")]
pub async fn open_space(_api_key: ApiKey, space: &State<SpaceGuard>) {
    space.open().await;
}

#[post("/admin/publish/space-close")]
pub async fn close_space(_api_key: ApiKey, space: &State<SpaceGuard>) {
    space.close().await;
}

#[derive(Debug, rocket::serde::Deserialize, rocket::serde::Serialize)]
pub struct KeepOpenResponse {
    /// Timestamp (UTC) till the space stays open
    pub open_till: u64,
}

#[post("/admin/publish/space-keep-open")]
pub async fn keep_open(_api_key: ApiKey, space: &State<SpaceGuard>) -> Json<KeepOpenResponse> {
    let till = space.keep_open().await;
    log::debug!("Space will be opened till {till:?}");
    Json(KeepOpenResponse {
        open_till: till.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
    })
}

#[derive(Debug, FromFormField)]
pub enum AdminUiControlAction {
    Open,
    Close,
}

impl AdminUiControlAction {
    pub fn to_str(&self) -> &'static str {
        match self {
            AdminUiControlAction::Open => "open",
            AdminUiControlAction::Close => "close",
        }
    }
}

#[derive(Debug, FromForm)]
pub struct AdminUiControlForm {
    api_key: String,
    action: AdminUiControlAction,
}

#[post("/admin/ui", data = "<data>")]
pub async fn admin_ui_control(
    data: Form<AdminUiControlForm>,
    space: &State<SpaceGuard>,
    api_key: &State<ApiKey>,
) -> Flash<Redirect> {
    if data.api_key == api_key.0 {
        let hint = match data.action {
            AdminUiControlAction::Open => {
                space.open().await;
                "Space is now open"
            }
            AdminUiControlAction::Close => {
                space.close().await;
                "Space is now closed"
            }
        };
        Flash::success(Redirect::to(uri!(admin_ui_view())), hint)
    } else {
        Flash::error(Redirect::to(uri!(admin_ui_control)), "Invalid API-Key")
    }
}

#[get("/admin/ui")]
pub async fn admin_ui_view(
    space: &State<SpaceGuard>,
    flash: Option<FlashMessage<'_>>,
) -> (ContentType, String) {
    let hint = flash
        .map(|msg| format!("<div>{}</div>", msg.message()))
        .unwrap_or("".to_string());
    let (command_title, command_value) = if space.is_open().await {
        ("🔐 Close space", AdminUiControlAction::Close.to_str())
    } else {
        ("🔓 Open space", AdminUiControlAction::Open.to_str())
    };
    let command_uri = uri!(admin_ui_control);

    let html = format!(
        r#"<html>
        <head>
            <meta charset="utf-8">
        </head>
        <body>
            {hint}
            <form action="{command_uri}" method="POST">
                <label for="api_key">API-Key</label>
                <input type="password" name="api_key"/>
                <input type="hidden" name="action" value="{command_value}"/>
                <br>
                <input type="submit" value="{command_title}" />
            </form>
        </body>
    </html>
    "#
    );
    (ContentType::HTML, html)
}

/// Minimalistic implementation of the index page
#[get("/")]
pub async fn index(
    space: &State<SpaceGuard>,
    displays: &State<StatusDisplayTypes>,
    template: &State<spaceapi_dezentrale::Status>,
) -> (ContentType, String) {
    let name = &template.space;
    let logo = &template.logo;
    let status = if space.is_open().await {
        &displays.text.open
    } else {
        &displays.text.closed
    };

    let html = format!(
        r#"<html>
        <body>
            <center>
                <img src="{logo}" alt="{name}"></img>
                <div>{status}</div>
                <div><a href="https://github.com/dezentrale/spaceapi-rs">{0} v{1}</a></div>
            </center>
        </body>
    </html>
    "#,
        crate::SOFTWARE,
        crate::VERSION,
    );
    (ContentType::HTML, html)
}

#[get("/spaceapi/v14")]
pub async fn get_status_v14<'a>(
    space: &State<SpaceGuard>,
    template: &State<spaceapi_dezentrale::Status>,
) -> Json<spaceapi_dezentrale::Status> {
    let mut status = template.inner().clone();
    status.api_compatibility = Some(vec![spaceapi_dezentrale::ApiVersion::V14]);
    status.state = Some(spaceapi_dezentrale::State {
        open: Some(space.is_open().await),
        lastchange: Some(crate::unix_timestamp()),
        ..spaceapi_dezentrale::State::default()
    });
    Json(status)
}

#[get("/status/text")]
pub async fn get_status_text<'a>(
    space: &State<SpaceGuard>,
    displays: &State<StatusDisplayTypes>,
) -> (ContentType, String) {
    let status = if space.is_open().await {
        displays.text.open.clone()
    } else {
        displays.text.closed.clone()
    };
    (ContentType::Text, status)
}

#[get("/status/html")]
pub async fn get_status_html<'a>(
    space: &State<SpaceGuard>,
    displays: &State<StatusDisplayTypes>,
) -> (ContentType, String) {
    let status = if space.is_open().await {
        displays.html.open.clone()
    } else {
        displays.html.closed.clone()
    };
    (ContentType::HTML, status)
}

/// OPTION fallback handler required for CORS
#[options("/<_..>")]
pub fn options_catch_all() {}
