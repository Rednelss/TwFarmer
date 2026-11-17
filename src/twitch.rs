use crate::{
    auth::{self, Auth, Identity, SessionExpired},
    hls,
    tracking::minute_watched_payload,
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use regex::Regex;
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::time::Instant;

const GQL: &str = "https://gql.twitch.tv/gql";
const POINTS_HASH: &str = "7fe050e3761eb2cf258d70ee1a21cbd76fa8cf3d7e7b12fc437e7029d446b5e3";
const CLAIM_HASH: &str = "46aaeebe02c99afdf4fc97c7c0cba964124bf6b0af229395f1f6d1feed05b3d0";
const PLAYBACK_HASH: &str = "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9";

pub fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0")
        .timeout(Duration::from_secs(25))
        .pool_idle_timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .gzip(true)
        .build()
        .context("Не удалось создать HTTP-клиент")
}

pub fn check_status(status: StatusCode, operation: &str) -> Result<()> {
    ensure!(status.is_success(), "{operation}: HTTP {status}");
    Ok(())
}

pub struct Twitch {
    pub client: Client,
    pub auth: Auth,
    pub identity: Identity,
    auth_path: PathBuf,
    session_id: String,
    tracking: Option<(Url, Instant)>,
}

#[derive(Clone)]
pub struct Channel {
    pub login: String,
    pub id: String,
    pub broadcast_id: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Points {
    pub balance: i64,
    #[serde(rename = "availableClaim")]
    pub available_claim: Option<Claim>,
}
#[derive(Deserialize, Debug)]
pub struct Claim {
    pub id: String,
}

pub struct Playback {
    pub broadcast_id: String,
    pub playlist: Url,
    pub expires: Instant,
}

impl Twitch {
    pub async fn connect(config: &crate::config::Config, auth_path: PathBuf) -> Result<Self> {
        let client = http_client()?;
        let mut auth = auth::read(&auth_path)?;
        ensure!(
            auth.client_id == config.client_id,
            "client_id изменился. Выполните twfarmer login."
        );
        let identity = auth::ensure_valid(&client, &mut auth, &auth_path).await?;
        Ok(Self {
            client,
            auth,
            identity,
            auth_path,
            session_id: uuid::Uuid::new_v4().simple().to_string(),
            tracking: None,
        })
    }

    pub async fn validate(&mut self) -> Result<()> {
        self.identity = auth::ensure_valid(&self.client, &mut self.auth, &self.auth_path).await?;
        Ok(())
    }

    async fn gql(&self, body: Value) -> Result<Value> {
        let response = self
            .client
            .post(GQL)
            .header("Authorization", format!("OAuth {}", self.auth.access_token))
            .header("Client-ID", &self.auth.client_id)
            .header("X-Device-ID", &self.auth.device_id)
            .header("Client-Session-ID", &self.session_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.without_url())
            .context("Запрос Twitch GraphQL")?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(SessionExpired.into());
        }
        check_status(response.status(), "Twitch GraphQL")?;
        let body: Value = response
            .json()
            .await
            .context("Неверный JSON Twitch GraphQL")?;
        gql_data(body)
    }

    async fn persisted(&self, name: &str, hash: &str, variables: Value) -> Result<Value> {
        self.gql(json!({
            "operationName": name, "variables": variables,
            "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } }
        }))
        .await
    }

    pub async fn channel(&self, login: &str) -> Result<Channel> {
        let data = self.gql(json!({
            "operationName":"Channel",
            "query":"query Channel($login: String!) { user(login: $login) { id login stream { id } } }",
            "variables": { "login": login }
        })).await?;
        let user = data
            .get("user")
            .filter(|u| !u.is_null())
            .with_context(|| format!("Канал {login} не найден"))?;
        let id = user["id"]
            .as_str()
            .context("Twitch не вернул ID канала")?
            .to_string();
        let broadcast_id = user
            .pointer("/stream/id")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(Channel {
            login: login.into(),
            id,
            broadcast_id,
        })
    }

    pub async fn points(&self, login: &str) -> Result<Option<Points>> {
        let data = self
            .persisted(
                "ChannelPointsContext",
                POINTS_HASH,
                json!({"channelLogin": login}),
            )
            .await?;
        parse_points(&data)
    }

    pub async fn claim(&self, channel: &Channel, claim: &Claim) -> Result<()> {
        let data = self
            .persisted(
                "ClaimCommunityPoints",
                CLAIM_HASH,
                json!({
                    "input": { "channelID": channel.id, "claimID": claim.id }
                }),
            )
            .await?;
        let result = data
            .get("claimCommunityPoints")
            .filter(|v| !v.is_null())
            .context("Twitch не подтвердил получение бонуса")?;
        if result.get("error").is_some_and(|e| !e.is_null()) {
            bail!("Twitch отклонил получение бонуса");
        }
        // Empty data or a null mutation is never reported as a successful claim.
        Ok(())
    }

    async fn get_text(&self, url: Url) -> Result<(Url, String)> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.without_url())
            .context("Загрузка данных потока")?;
        check_status(response.status(), "Загрузка данных потока")?;
        let final_url = response.url().clone();
        Ok((
            final_url,
            response
                .text()
                .await
                .map_err(|e| e.without_url())
                .context("Чтение данных потока")?,
        ))
    }

    async fn tracking_url(&mut self) -> Result<Url> {
        if let Some((url, expires)) = &self.tracking
            && Instant::now() < *expires
        {
            return Ok(url.clone());
        }
        let (_, html) = self.get_text(Url::parse("https://www.twitch.tv/")?).await?;
        let re = Regex::new(
            r#"https://(?:static\.twitchcdn\.net|assets\.twitch\.tv)/config/settings[^"<>\s]+\.js"#,
        )?;
        let settings = re
            .find(&html)
            .context("Не найден settings.js Twitch; внутренний API изменился")?
            .as_str();
        let (_, text) = self.get_text(Url::parse(settings)?).await?;
        let url = tracking_endpoint(&text)?;
        self.tracking = Some((url.clone(), Instant::now() + Duration::from_secs(3600)));
        Ok(url)
    }

    pub async fn playback(&self, channel: &Channel) -> Result<Playback> {
        let data = self
            .persisted(
                "PlaybackAccessToken",
                PLAYBACK_HASH,
                json!({
                    "login":channel.login, "isLive":true, "isVod":false, "vodID":"",
                    "playerType":"site", "platform":"web"
                }),
            )
            .await?;
        let token = &data["streamPlaybackAccessToken"];
        let signature = token["signature"]
            .as_str()
            .context("Нет подписи доступа к потоку")?;
        let value = token["value"]
            .as_str()
            .context("Нет токена доступа к потоку")?;
        let mut url = Url::parse(&format!(
            "https://usher.ttvnw.net/api/channel/hls/{}.m3u8",
            channel.login
        ))?;
        url.query_pairs_mut()
            .append_pair("sig", signature)
            .append_pair("token", value)
            .append_pair("allow_source", "true")
            .append_pair("allow_audio_only", "true")
            .append_pair("player", "twitchweb");
        let (base, playlist) = self.get_text(url).await?;
        Ok(Playback {
            broadcast_id: channel.broadcast_id.clone().context("Канал не в эфире")?,
            playlist: hls::lowest_variant(&base, &playlist)?,
            expires: Instant::now() + Duration::from_secs(240),
        })
    }

    pub async fn watch(&mut self, channel: &Channel, playback: &Playback) -> Result<()> {
        let (base, playlist) = self.get_text(playback.playlist.clone()).await?;
        let segment = hls::latest_segment(&base, &playlist)?;
        let mut response = self
            .client
            .get(segment)
            .send()
            .await
            .map_err(|e| e.without_url())
            .context("Получение сегмента потока")?;
        check_status(response.status(), "Сегмент потока")?;
        // Consume one low-quality segment per tick without buffering the video in RAM.
        let mut size = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| e.without_url())
            .context("Чтение сегмента")?
        {
            size += chunk.len();
            ensure!(size <= 16 * 1024 * 1024, "Сегмент потока превышает 16 МиБ");
        }
        ensure!(size > 0, "Twitch вернул пустой сегмент потока");
        let payload = minute_watched_payload(channel, &self.identity.user_id)?;
        self.send_tracking(&payload).await?;
        Ok(())
    }

    async fn send_tracking(&mut self, payload: &Value) -> Result<()> {
        let encoded = STANDARD.encode(serde_json::to_vec(payload)?);
        let endpoint = self.tracking_url().await?;
        let operation = format!(
            "Событие просмотра ({})",
            endpoint.host_str().unwrap_or("Twitch")
        );
        let response = self
            .client
            .post(endpoint)
            .form(&[("data", encoded)])
            .send()
            .await
            .map_err(|error| {
                self.tracking = None;
                error.without_url()
            })
            .with_context(|| operation.clone())?;
        if !response.status().is_success() {
            self.tracking = None;
        }
        check_status(response.status(), &operation)?;
        // HTTP success confirms delivery, never that points were awarded.
        Ok(())
    }
}

#[derive(Deserialize)]
struct TrackingSettings {
    #[serde(default)]
    use_beacon_url: bool,
    beacon_url: Option<String>,
    spade_url: Option<String>,
}

fn tracking_endpoint(script: &str) -> Result<Url> {
    let start = script
        .find('{')
        .context("В settings.js Twitch нет настроек")?;
    // The JSON object may be followed by a semicolon or additional JavaScript.
    let settings = serde_json::Deserializer::from_str(&script[start..])
        .into_iter::<TrackingSettings>()
        .next()
        .context("В settings.js Twitch нет настроек")?
        .context("Не удалось разобрать настройки событий Twitch")?;
    let (key, value) = if settings.use_beacon_url {
        ("beacon_url", settings.beacon_url)
    } else {
        ("spade_url", settings.spade_url)
    };
    let value = value.with_context(|| format!("В настройках Twitch отсутствует {key}"))?;
    let url = Url::parse(&value).context("Неверный адрес событий Twitch")?;
    ensure!(
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|h| h == "twitch.tv" || h.ends_with(".twitch.tv")),
        "Неверный адрес событий Twitch"
    );
    Ok(url)
}

pub fn parse_points(data: &Value) -> Result<Option<Points>> {
    let community = data
        .get("community")
        .filter(|v| !v.is_null())
        .context("Twitch не вернул канал в контексте баллов")?;
    if community
        .pointer("/channel/communityPointsSettings/isEnabled")
        .and_then(Value::as_bool)
        == Some(false)
    {
        return Ok(None);
    }
    let points = community
        .pointer("/channel/self/communityPoints")
        .filter(|v| !v.is_null())
        .context("Twitch не вернул баланс: баллы недоступны для этой сессии")?;
    Ok(Some(
        serde_json::from_value(points.clone()).context("Формат баллов Twitch изменился")?,
    ))
}

fn gql_data(body: Value) -> Result<Value> {
    if let Some(errors) = body.get("errors").and_then(Value::as_array)
        && !errors.is_empty()
    {
        let messages: Vec<_> = errors
            .iter()
            .filter_map(|e| e["message"].as_str())
            .collect();
        let message = messages.join(" ").to_ascii_lowercase();
        if message.contains("unauth")
            || message.contains("invalid oauth")
            || message.contains("invalid token")
        {
            return Err(SessionExpired.into());
        }
        if message.contains("persistedquerynotfound") {
            bail!("Запрос Twitch устарел (PersistedQueryNotFound); требуется обновление программы");
        }
        bail!("Twitch вернул ошибку GraphQL; внутренний API может быть недоступен или изменён");
    }
    body.get("data")
        .filter(|v| !v.is_null())
        .cloned()
        .context("Twitch не вернул данные GraphQL")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_beacon_when_enabled_by_twitch() {
        let settings = r#"window.__twilightSettings = {"use_beacon_url":true,"beacon_url":"https://beacon.twitch.tv/track","spade_url":"https://spade.twitch.tv/track"};"#;
        assert_eq!(
            tracking_endpoint(settings).unwrap().as_str(),
            "https://beacon.twitch.tv/track"
        );
    }

    #[test]
    fn legacy_settings_keep_spade_when_beacon_is_disabled_or_absent() {
        for settings in [
            r#"{"use_beacon_url":false,"beacon_url":"https://beacon.twitch.tv/track","spade_url":"https://spade.twitch.tv/track"}"#,
            r#"window.__twilightSettings={"spade_url":"https:\/\/spade.twitch.tv\/track"};"#,
        ] {
            assert_eq!(
                tracking_endpoint(settings).unwrap().as_str(),
                "https://spade.twitch.tv/track"
            );
        }
    }

    #[test]
    fn rejects_missing_enabled_endpoint_and_untrusted_urls() {
        for settings in [
            r#"{"use_beacon_url":true,"spade_url":"https://spade.twitch.tv/track"}"#,
            r#"{"use_beacon_url":true,"beacon_url":"http://beacon.twitch.tv/track"}"#,
            r#"{"use_beacon_url":true,"beacon_url":"https://twitch.tv.example.org/track"}"#,
            r#"window.__twilightSettings = not_json;"#,
        ] {
            assert!(tracking_endpoint(settings).is_err(), "{settings}");
        }
    }

    #[test]
    fn parses_claim_and_balance() {
        let data = json!({"community":{"channel":{"communityPointsSettings":{"isEnabled":true},"self":{"communityPoints":{"balance":120,"availableClaim":{"id":"bonus"}}}}}});
        let points = parse_points(&data).unwrap().unwrap();
        assert_eq!(points.balance, 120);
        assert_eq!(points.available_claim.unwrap().id, "bonus");
    }
    #[test]
    fn unavailable_balance_is_not_a_zero_balance() {
        assert!(
            parse_points(&json!({"community":{"channel":{"self":{"communityPoints":null}}}}))
                .is_err()
        );
        assert!(
            parse_points(
                &json!({"community":{"channel":{"communityPointsSettings":{"isEnabled":false}}}})
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn graphql_errors_are_not_reported_as_success() {
        assert!(
            gql_data(json!({"data":{"user":null},"errors":[{"message":"PersistedQueryNotFound"}]}))
                .unwrap_err()
                .to_string()
                .contains("устарел")
        );
        assert!(
            gql_data(json!({"errors":[{"message":"Unauthorized"}]}))
                .unwrap_err()
                .is::<SessionExpired>()
        );
        assert!(gql_data(json!({"data":null})).is_err());
    }
}
