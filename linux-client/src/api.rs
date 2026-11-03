//! Hopp's authenticated HTTP API. Media tokens never appear in errors or Debug output.

use anyhow::{Result, bail};
use reqwest::{Client, Method, StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::time::Duration;

pub const DEFAULT_API_BASE: &str = "https://pair.gethopp.app";

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    base: Url,
    token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Teammate {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    #[serde(default)]
    pub is_active: bool,
}

pub type RoomsPresence = HashMap<String, Vec<String>>;

#[derive(Deserialize)]
struct PresenceResponse {
    rooms: RoomsPresence,
}

#[derive(Debug, Clone)]
pub struct JoinCallResult {
    pub tokens: RoomTokens,
    pub room: Option<RoomInfo>,
}

#[derive(Clone)]
pub struct RoomTokens {
    pub audio_token: String,
    pub video_token: String,
    pub livekit_url: String,
    pub participant: String,
}

impl std::fmt::Debug for RoomTokens {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoomTokens")
            .field("audio_token", &"[redacted]")
            .field("video_token", &"[redacted]")
            .field("livekit_url", &"[configured]")
            .field("participant", &self.participant)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenResponse {
    audio_token: String,
    video_token: String,
    participant: String,
}

impl RoomTokens {
    /// The websocket call_tokens payload uses the same token keys as GET room.
    /// `participant` is the remote user for direct calls, and self for named rooms.
    pub fn from_payload(payload: serde_json::Value, livekit_url: String) -> Result<Self> {
        validate_livekit_url(&livekit_url)?;
        let tokens: TokenResponse = serde_json::from_value(payload)
            .map_err(|_| anyhow::anyhow!("Hopp server returned an unexpected call response"))?;
        if tokens.audio_token.is_empty()
            || tokens.video_token.is_empty()
            || tokens.participant.is_empty()
        {
            bail!("Hopp server returned incomplete call credentials");
        }
        Ok(Self {
            audio_token: tokens.audio_token,
            video_token: tokens.video_token,
            livekit_url,
            participant: tokens.participant,
        })
    }
}

#[derive(Deserialize)]
struct ServerResponse {
    url: String,
}

fn validate_livekit_url(value: &str) -> Result<()> {
    let livekit = Url::parse(value)
        .map_err(|_| anyhow::anyhow!("Hopp server returned an invalid media address"))?;
    if !matches!(livekit.scheme(), "ws" | "wss")
        || livekit.host_str().is_none()
        || !livekit.username().is_empty()
        || livekit.password().is_some()
    {
        bail!("Hopp server returned an invalid media address");
    }
    Ok(())
}

fn server_base(value: &str) -> Result<Url> {
    let value = value.trim();
    let qualified = if value.contains("://") {
        value.to_string()
    } else {
        format!("https://{value}")
    };
    let mut base =
        Url::parse(&qualified).map_err(|_| anyhow::anyhow!("Invalid Hopp server address"))?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || base.path() != "/"
    {
        bail!("Hopp server must be an HTTP or HTTPS origin without credentials, query, or path");
    }
    base.set_path("/");
    Ok(base)
}

/// Login remains in the system browser, including social login and verification.
pub fn login_url(api_base: &str) -> Result<String> {
    Ok(server_base(api_base)?.join("login-app")?.to_string())
}

/// Hopp's browser returns `hopp:///authenticate?token=...`.
/// Also accept the authority form some desktop launchers normalize it to.
pub fn parse_auth_deep_link(value: &str) -> Result<String> {
    let uri = Url::parse(value).map_err(|_| anyhow::anyhow!("Invalid Hopp sign-in link"))?;
    let valid_route = match uri.host_str() {
        None => uri.path() == "/authenticate",
        Some("authenticate") => matches!(uri.path(), "" | "/"),
        _ => false,
    };
    if uri.scheme() != "hopp"
        || !valid_route
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.port().is_some()
        || uri.fragment().is_some()
    {
        bail!("This is not a Hopp sign-in link");
    }
    let tokens: Vec<_> = uri
        .query_pairs()
        .filter(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned())
        .collect();
    if tokens.len() != 1
        || tokens[0].trim().is_empty()
        || tokens[0].chars().any(char::is_whitespace)
        || tokens[0].chars().any(char::is_control)
    {
        bail!("Hopp sign-in link needs one nonempty token");
    }
    Ok(tokens.into_iter().next().expect("one token checked above"))
}

impl ApiClient {
    pub fn new(api_base: &str, token: Option<String>) -> Result<Self> {
        let base = server_base(api_base)?;
        let token = token.map(|value| value.trim().to_owned());
        if token.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.chars().any(char::is_whitespace)
                || value.chars().any(char::is_control)
        }) {
            bail!("Hopp token must be nonempty and contain no whitespace");
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| anyhow::anyhow!("Could not initialize Hopp networking"))?;
        Ok(Self {
            client,
            base,
            token,
        })
    }

    #[cfg(test)]
    pub fn login_url(&self) -> String {
        self.base
            .join("login-app")
            .expect("constant relative URL")
            .to_string()
    }

    pub fn websocket_url(&self) -> Result<Url> {
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Sign in to Hopp first"))?;
        let mut url = self.base.join("api/auth/websocket")?;
        let scheme = if self.base.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        url.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("Invalid Hopp websocket address"))?;
        url.query_pairs_mut().append_pair("token", token);
        Ok(url)
    }

    async fn request(
        &self,
        method: Method,
        endpoint: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Sign in to Hopp first"))?;
        let mut request = self
            .client
            .request(method, self.base.join(endpoint)?)
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                anyhow::anyhow!("Hopp server timed out")
            } else {
                anyhow::anyhow!("Could not reach the Hopp server; check its address and connection")
            }
        })?;
        match response.status() {
            status if status.is_success() => Ok(response),
            StatusCode::UNAUTHORIZED => {
                bail!("Hopp sign-in expired or the token is invalid; sign in again")
            }
            StatusCode::FORBIDDEN => bail!("Your Hopp account does not have access to this room"),
            StatusCode::PAYMENT_REQUIRED => {
                bail!("Your Hopp subscription or trial does not allow joining this room")
            }
            StatusCode::NOT_FOUND => bail!("The Hopp room or endpoint was not found"),
            status => bail!("Hopp server returned HTTP {}", status.as_u16()),
        }
    }

    async fn get<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        self.request(Method::GET, endpoint, None)
            .await?
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("Hopp server returned an unexpected response"))
    }

    fn room_endpoint(id: &str, leave: bool) -> Result<String> {
        if id.is_empty()
            || !id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            bail!("Invalid Hopp room ID");
        }
        Ok(format!(
            "api/auth/room/{id}{}",
            if leave { "/leave" } else { "" }
        ))
    }

    pub async fn user(&self) -> Result<UserInfo> {
        self.get("api/auth/user").await
    }

    pub async fn teammates(&self) -> Result<Vec<Teammate>> {
        let mut teammates: Vec<Teammate> = self.get("api/auth/teammates").await?;
        teammates.sort_by_key(|teammate| teammate.first_name.to_lowercase());
        Ok(teammates)
    }

    pub async fn rooms_presence(&self) -> Result<RoomsPresence> {
        Ok(self
            .get::<PresenceResponse>("api/auth/rooms/presence")
            .await?
            .rooms)
    }

    pub async fn server_url(&self) -> Result<String> {
        let server: ServerResponse = self.get("api/auth/livekit/server-url").await?;
        validate_livekit_url(&server.url)?;
        Ok(server.url)
    }

    /// Join a teammate's existing call, including an accepted invitation.
    /// Named-room details are present only when the existing call is a DB room.
    pub async fn join_call(&self, user_id: &str) -> Result<JoinCallResult> {
        Self::room_endpoint(user_id, false)?; // IDs have the same safe UUID syntax.
        let livekit_url = self.server_url().await?;
        let payload: serde_json::Value = self
            .request(Method::POST, &format!("api/auth/call/join/{user_id}"), None)
            .await?
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("Hopp server returned an unexpected call response"))?;
        let room = match payload.get("room") {
            None | Some(serde_json::Value::Null) => None,
            Some(room) => Some(
                serde_json::from_value(room.clone())
                    .map_err(|_| anyhow::anyhow!("Hopp server returned an unexpected room"))?,
            ),
        };
        Ok(JoinCallResult {
            tokens: RoomTokens::from_payload(payload, livekit_url)?,
            room,
        })
    }

    pub async fn rooms(&self) -> Result<Vec<RoomInfo>> {
        let mut rooms: Vec<RoomInfo> = self.get("api/auth/rooms").await?;
        rooms.sort_by_key(|room| room.name.to_lowercase());
        Ok(rooms)
    }

    pub async fn create_room(&self, name: &str) -> Result<RoomInfo> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Enter a room name");
        }
        self.request(
            Method::POST,
            "api/auth/room",
            Some(serde_json::json!({"name": name})),
        )
        .await?
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("Hopp server returned an unexpected room"))
    }

    pub async fn join_room(&self, id: &str) -> Result<RoomTokens> {
        let endpoint = Self::room_endpoint(id, false)?;
        // Fetch/validate configuration first: fetching room tokens already marks us in the room.
        let livekit_url = self.server_url().await?;
        let response = self.request(Method::GET, &endpoint, None).await?;
        let tokens: TokenResponse = match response.json().await {
            Ok(tokens) => tokens,
            Err(_) => {
                // A malformed successful response occurs after presence changed server-side.
                let _ = self.leave_room(id).await;
                bail!("Hopp server returned an unexpected room response");
            }
        };
        if tokens.audio_token.is_empty()
            || tokens.video_token.is_empty()
            || tokens.participant.is_empty()
        {
            let _ = self.leave_room(id).await;
            bail!("Hopp server returned incomplete room credentials");
        }
        Ok(RoomTokens {
            audio_token: tokens.audio_token,
            video_token: tokens.video_token,
            livekit_url,
            participant: tokens.participant,
        })
    }

    pub async fn leave_room(&self, id: &str) -> Result<()> {
        self.request(Method::POST, &Self::room_endpoint(id, true)?, None)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticates_only_hopp_sign_in_links() {
        assert_eq!(
            parse_auth_deep_link("hopp:///authenticate?token=abc.def.ghi").unwrap(),
            "abc.def.ghi"
        );
        assert_eq!(
            parse_auth_deep_link("hopp://authenticate?token=abc").unwrap(),
            "abc"
        );
        for uri in [
            "https://example.com/authenticate?token=abc",
            "hopp:///slack-join?token=abc",
            "hopp://evil/authenticate?token=abc",
            "hopp:///authenticate?token=",
            "hopp:///authenticate?token=one&token=two",
            "hopp:///authenticate?token=a%0Ab",
            "hopp:///authenticate?token=abc#other",
        ] {
            assert!(
                parse_auth_deep_link(uri).is_err(),
                "unexpectedly accepted {uri}"
            );
        }
    }

    #[test]
    fn builds_login_and_presence_urls_from_configured_origin() {
        assert_eq!(
            login_url("pair.gethopp.app").unwrap(),
            "https://pair.gethopp.app/login-app"
        );
        let api = ApiClient::new("http://localhost:8080", Some("abc.def.ghi".into())).unwrap();
        assert_eq!(api.login_url(), "http://localhost:8080/login-app");
        assert_eq!(
            api.websocket_url().unwrap().as_str(),
            "ws://localhost:8080/api/auth/websocket?token=abc.def.ghi"
        );
        for origin in [
            "https://user:password@example.com",
            "https://example.com/?token=secret",
            "https://example.com/api",
            "file:///tmp/hopp",
            "",
        ] {
            assert!(ApiClient::new(origin, None).is_err());
        }
    }

    #[test]
    fn reads_hopp_actual_response_shapes() {
        let response: TokenResponse = serde_json::from_str(
            r#"{"audioToken":"a","videoToken":"v","cameraToken":"c","participant":"usr-123"}"#,
        )
        .unwrap();
        assert_eq!(response.audio_token, "a");
        assert_eq!(response.video_token, "v");
        assert_eq!(response.participant, "usr-123");
        let room: RoomInfo = serde_json::from_str(
            r#"{"id":"019a-123","name":"Watercooler","team_id":1,"temp":false}"#,
        )
        .unwrap();
        assert_eq!(room.name, "Watercooler");
        assert!(ApiClient::room_endpoint("../../user", false).is_err());
        let teammate: Teammate = serde_json::from_str(r#"{"id":"usr-456","first_name":"Ada","last_name":"Local","email":"ada@hopp-native.invalid","is_active":true,"team_id":7,"avatar_url":null}"#).unwrap();
        assert!(teammate.is_active);
        assert_eq!(teammate.first_name, "Ada");
        let presence: PresenceResponse =
            serde_json::from_str(r#"{"rooms":{"room-123":["usr-456"]}}"#).unwrap();
        assert_eq!(presence.rooms["room-123"], ["usr-456"]);
        let tokens = RoomTokens::from_payload(serde_json::json!({"audioToken":"a", "videoToken":"v", "cameraToken":"c", "participant":"other-user"}), "ws://127.0.0.1:18880".into()).unwrap();
        assert_eq!(tokens.participant, "other-user");
        assert!(
            RoomTokens::from_payload(
                serde_json::json!({"audioToken":"", "videoToken":"v", "participant":"peer"}),
                "ws://127.0.0.1:18880".into()
            )
            .is_err()
        );
    }
}
