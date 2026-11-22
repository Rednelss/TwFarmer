use crate::{
    auth::{self, Auth, Identity, SessionExpired},
    hls,
    tracking::minute_watched_payload,
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use regex::Regex;
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::LazyLock,
    time::Duration,
};
use tokio::time::Instant;

const GQL: &str = "https://gql.twitch.tv/gql";
const POINTS_HASH: &str = "7fe050e3761eb2cf258d70ee1a21cbd76fa8cf3d7e7b12fc437e7029d446b5e3";
const CLAIM_HASH: &str = "46aaeebe02c99afdf4fc97c7c0cba964124bf6b0af229395f1f6d1feed05b3d0";
const PLAYBACK_HASH: &str = "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9";
static SETTINGS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"https://(?:static\.twitchcdn\.net|assets\.twitch\.tv)/config/settings[^"<>\s]+\.js"#,
    )
    .expect("valid Twitch settings URL pattern")
});

pub fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0")
        .timeout(Duration::from_secs(25))
        .pool_idle_timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .gzip(true)
        .build()
        .context(tr!(
            "Could not create the HTTP client",
            "Не удалось создать HTTP-клиент"
        ))
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
    pub game_id: Option<String>,
    pub game_name: Option<String>,
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
            "{}",
            tf!(
                "client_id changed. Run twfarmer login.",
                "client_id изменился. Выполните twfarmer login."
            )
        );
        let identity = auth::ensure_valid(&client, &mut auth, &auth_path).await?;
        auth::sync_identity(&mut auth, &identity, &auth_path)?;
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
        auth::sync_identity(&mut self.auth, &self.identity, &self.auth_path)?;
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
            .context(tr!("Twitch GraphQL request", "Запрос Twitch GraphQL"))?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(SessionExpired.into());
        }
        check_status(response.status(), "Twitch GraphQL")?;
        let body: Value = response.json().await.context(tr!(
            "Invalid Twitch GraphQL JSON",
            "Неверный JSON Twitch GraphQL"
        ))?;
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
            "query":"query Channel($login: String!) { user(login: $login) { id login stream { id game { id name } } } }",
            "variables": { "login": login }
        })).await?;
        let user = data
            .get("user")
            .filter(|u| !u.is_null())
            .with_context(|| tf!("Channel {login} was not found", "Канал {login} не найден"))?;
        let id = user["id"]
            .as_str()
            .context(tr!(
                "Twitch did not return a channel ID",
                "Twitch не вернул ID канала"
            ))?
            .to_string();
        let broadcast_id = user
            .pointer("/stream/id")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(Channel {
            login: login.into(),
            id,
            broadcast_id,
            game_id: user
                .pointer("/stream/game/id")
                .and_then(Value::as_str)
                .map(str::to_string),
            game_name: user
                .pointer("/stream/game/name")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    pub async fn drops(&self, channel: &Channel) -> Result<Vec<crate::drops::Drop>> {
        let data = self
            .gql(json!({
                "operationName": "DropChannelCampaignsProgress",
                "query": crate::drops::channel_query(),
                "variables": {"channelID": channel.id}
            }))
            .await?;
        crate::drops::parse(&data, &channel.id, channel.game_id.as_deref())
    }

    pub async fn available_drops(&self) -> Result<Vec<crate::drops::CatalogEntry>> {
        let data = self
            .gql(json!({"query": "query { dropsCampaigns(includeUpcoming: false) { id status } }"}))
            .await?;
        let campaigns = data.get("dropsCampaigns").context(tr!(
            "Twitch did not return available campaigns",
            "Twitch не вернул доступные кампании"
        ))?;
        let ids: Vec<String> = campaigns
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["status"].as_str() == Some("ACTIVE"))
            .filter_map(|c| c["id"].as_str().map(str::to_string))
            .collect();
        // Catalog rows are public; details resolve the viewer's progress and account links.
        let chunks: Vec<Vec<String>> = ids.chunks(10).map(|chunk| chunk.to_vec()).collect();
        let results: Vec<Result<Vec<crate::drops::Campaign>>> = stream::iter(chunks)
            .map(|ids| async move {
                let data = self.gql(json!({"operationName": "AvailableDropsCampaigns", "query": crate::drops::details_query(&ids)})).await?;
                let values: Vec<Value> = (0..ids.len()).filter_map(|i| data.get(format!("campaign_{i}")).filter(|c| !c.is_null()).cloned()).collect();
                crate::drops::parse_campaigns(&json!(values))
            }).buffered(3).collect().await;
        let mut campaigns = vec![];
        for result in results {
            campaigns.extend(result?);
        }
        Ok(crate::drops::catalog(campaigns))
    }

    async fn directory(&self, game_id: Option<&str>) -> Result<Vec<String>> {
        let query = if game_id.is_some() {
            "query DropsDirectory($gameID: ID!) { game(id: $gameID) { streams(first: 100, options: {systemFilters: [DROPS_ENABLED]}) { edges { node { broadcaster { login } } } } } }"
        } else {
            "query DropsDirectory { streams(first: 100) { edges { node { broadcaster { login } } } } }"
        };
        let data = self.gql(json!({"operationName": "DropsDirectory", "query": query, "variables": {"gameID": game_id}})).await?;
        let pointer = if game_id.is_some() {
            "/game/streams/edges"
        } else {
            "/streams/edges"
        };
        Ok(data
            .pointer(pointer)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|edge| {
                edge.pointer("/node/broadcaster/login")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    pub async fn find_drop_channel(
        &self,
        catalog: &[crate::drops::CatalogEntry],
        excluded: &HashSet<String>,
    ) -> Result<Option<(Channel, Vec<crate::drops::Drop>)>> {
        let mut directories: HashMap<Option<String>, Vec<String>> = HashMap::new();
        let mut checked = HashSet::new();
        for entry in catalog.iter().filter(|entry| entry.drop.pending()) {
            let mut logins = entry.channels.clone();
            if logins.is_empty() {
                let categories: Vec<_> = if entry.categories.is_empty() {
                    vec![None]
                } else {
                    entry.categories.iter().cloned().map(Some).collect()
                };
                for category in categories {
                    if !directories.contains_key(&category) {
                        directories
                            .insert(category.clone(), self.directory(category.as_deref()).await?);
                    }
                    logins.extend(directories[&category].clone());
                }
            }
            for login in logins {
                if excluded.contains(&login) || !checked.insert(login.clone()) {
                    continue;
                }
                let channel = self.channel(&login).await?;
                if channel.broadcast_id.is_none() {
                    continue;
                }
                let drops = self.drops(&channel).await?;
                if drops.iter().any(|d| {
                    d.pending()
                        && catalog.iter().any(|e| {
                            e.drop.pending()
                                && d.campaign_id == e.drop.campaign_id
                                && d.group_id == e.drop.group_id
                        })
                }) {
                    return Ok(Some((channel, drops)));
                }
            }
        }
        Ok(None)
    }

    pub async fn claim_drop(&self, drop: &crate::drops::Drop) -> Result<()> {
        let id = drop.claim_id(&self.identity.user_id).context(tr!(
            "The Drop is not ready to claim",
            "Drop ещё нельзя забрать"
        ))?;
        let data = self
            .persisted(
                "DropsPage_ClaimDropRewards",
                "a455deea71bdc9015b78eb49f4acfbce8baa7ccbedd28e549bb025bd0f751930",
                json!({"input": {"dropInstanceID": id}}),
            )
            .await?;
        crate::drops::confirm_claim(&data)
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
            .context(tr!(
                "Twitch did not confirm the bonus claim",
                "Twitch не подтвердил получение бонуса"
            ))?;
        if result.get("error").is_some_and(|e| !e.is_null()) {
            bail!(
                "{}",
                tf!(
                    "Twitch rejected the bonus claim",
                    "Twitch отклонил получение бонуса"
                )
            );
        }
        // A null mutation does not confirm a bonus claim.
        Ok(())
    }

    async fn get_text(&self, url: Url) -> Result<(Url, String)> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.without_url())
            .context(tr!("Loading stream data", "Загрузка данных потока"))?;
        check_status(
            response.status(),
            tr!("Loading stream data", "Загрузка данных потока"),
        )?;
        let final_url = response.url().clone();
        Ok((
            final_url,
            response
                .text()
                .await
                .map_err(|e| e.without_url())
                .context(tr!("Reading stream data", "Чтение данных потока"))?,
        ))
    }

    async fn tracking_url(&mut self) -> Result<Url> {
        if let Some((url, expires)) = &self.tracking
            && Instant::now() < *expires
        {
            return Ok(url.clone());
        }
        let (_, html) = self.get_text(Url::parse("https://www.twitch.tv/")?).await?;
        let settings = SETTINGS_URL
            .find(&html)
            .context(tr!(
                "Twitch settings.js was not found; the internal API changed",
                "Не найден settings.js Twitch; внутренний API изменился"
            ))?
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
        let signature = token["signature"].as_str().context(tr!(
            "Missing stream access signature",
            "Нет подписи доступа к потоку"
        ))?;
        let value = token["value"].as_str().context(tr!(
            "Missing stream access token",
            "Нет токена доступа к потоку"
        ))?;
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
            broadcast_id: channel
                .broadcast_id
                .clone()
                .context(tr!("The channel is offline", "Канал не в эфире"))?,
            playlist: hls::lowest_variant(&base, &playlist)?,
            expires: Instant::now() + Duration::from_secs(240),
        })
    }

    pub async fn watch(
        &mut self,
        channel: &Channel,
        playback: &Playback,
        farm_drops: bool,
    ) -> Result<()> {
        let (base, playlist) = self.get_text(playback.playlist.clone()).await?;
        if farm_drops {
            // Drops count the playlist's segments, not only its latest segment.
            for segment in hls::segments(&base, &playlist)? {
                let response = self
                    .client
                    .head(segment)
                    .send()
                    .await
                    .map_err(|e| e.without_url())
                    .context(tr!("Drops stream segment", "Сегмент потока Drops"))?;
                check_status(
                    response.status(),
                    tr!("Drops stream segment", "Сегмент потока Drops"),
                )?;
            }
        }
        let segment = hls::latest_segment(&base, &playlist)?;
        let mut response = self
            .client
            .get(segment)
            .send()
            .await
            .map_err(|e| e.without_url())
            .context(tr!(
                "Fetching a stream segment",
                "Получение сегмента потока"
            ))?;
        check_status(response.status(), tr!("Stream segment", "Сегмент потока"))?;
        // Read one segment per tick; discard chunks instead of buffering video.
        let mut size = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| e.without_url())
            .context(tr!("Reading a segment", "Чтение сегмента"))?
        {
            size += chunk.len();
            ensure!(
                size <= 16 * 1024 * 1024,
                "{}",
                tf!(
                    "The stream segment exceeds 16 MiB",
                    "Сегмент потока превышает 16 МиБ"
                )
            );
        }
        ensure!(
            size > 0,
            "{}",
            tf!(
                "Twitch returned an empty stream segment",
                "Twitch вернул пустой сегмент потока"
            )
        );
        let payload = minute_watched_payload(channel, &self.identity.user_id)?;
        self.send_tracking(&payload).await?;
        Ok(())
    }

    async fn send_tracking(&mut self, payload: &Value) -> Result<()> {
        let encoded = STANDARD.encode(serde_json::to_vec(payload)?);
        let endpoint = self.tracking_url().await?;
        let operation = tf!(
            "Watch event ({})",
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
    let start = script.find('{').context(tr!(
        "Twitch settings.js contains no settings",
        "В settings.js Twitch нет настроек"
    ))?;
    // Ignore JavaScript after the settings JSON.
    let settings = serde_json::Deserializer::from_str(&script[start..])
        .into_iter::<TrackingSettings>()
        .next()
        .context(tr!(
            "Twitch settings.js contains no settings",
            "В settings.js Twitch нет настроек"
        ))?
        .context(tr!(
            "Could not parse Twitch event settings",
            "Не удалось разобрать настройки событий Twitch"
        ))?;
    let (key, value) = if settings.use_beacon_url {
        ("beacon_url", settings.beacon_url)
    } else {
        ("spade_url", settings.spade_url)
    };
    let value = value.with_context(|| {
        tf!(
            "Twitch settings do not contain {key}",
            "В настройках Twitch отсутствует {key}"
        )
    })?;
    let url = Url::parse(&value).context(tr!(
        "Invalid Twitch event URL",
        "Неверный адрес событий Twitch"
    ))?;
    ensure!(
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|h| h == "twitch.tv" || h.ends_with(".twitch.tv")),
        "{}",
        tf!("Invalid Twitch event URL", "Неверный адрес событий Twitch")
    );
    Ok(url)
}

pub fn parse_points(data: &Value) -> Result<Option<Points>> {
    let community = data.get("community").filter(|v| !v.is_null()).context(tr!(
        "Twitch did not return a channel in the points context",
        "Twitch не вернул канал в контексте баллов"
    ))?;
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
        .context(tr!(
            "Twitch did not return a balance: points are unavailable for this session",
            "Twitch не вернул баланс: баллы недоступны для этой сессии"
        ))?;
    Ok(Some(serde_json::from_value(points.clone()).context(
        tr!(
            "The Twitch points format changed",
            "Формат баллов Twitch изменился"
        ),
    )?))
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
            bail!(
                "{}",
                tf!(
                    "The Twitch query is outdated (PersistedQueryNotFound); update the program",
                    "Запрос Twitch устарел (PersistedQueryNotFound); требуется обновление программы"
                )
            );
        }
        bail!(
            "{}",
            tf!(
                "Twitch returned a GraphQL error; the internal API may be unavailable or changed",
                "Twitch вернул ошибку GraphQL; внутренний API может быть недоступен или изменён"
            )
        );
    }
    body.get("data")
        .filter(|v| !v.is_null())
        .cloned()
        .context(tr!(
            "Twitch did not return GraphQL data",
            "Twitch не вернул данные GraphQL"
        ))
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
                .contains("outdated")
        );
        assert!(
            gql_data(json!({"errors":[{"message":"Unauthorized"}]}))
                .unwrap_err()
                .is::<SessionExpired>()
        );
        assert!(gql_data(json!({"data":null})).is_err());
    }
}
