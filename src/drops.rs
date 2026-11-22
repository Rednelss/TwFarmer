use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

pub const FIELDS: &str = r#"
    id name startAt endAt
    rewardGroups {
      id name
      progressCriteria {
        requirementType isAutoProgressable
        requirements { minutesWatched subs turboSubs }
        channels { id login } categories { id }
        repeatableConfig { repeatableTimes }
      }
      rewards {
        name distributionType isAccountConnected accountLinkURL
        pool { selectionMode }
      }
      self {
        status hasPreconditionsMet currentMinutesWatched currentSubs currentTurboSubs grantCount nextEligibleAt
        currentWindow { expiresAt }
      }
    }
"#;

pub fn channel_query() -> String {
    format!(
        "query DropChannelCampaignsProgress($channelID: ID!) {{ channelDropCampaignsProgress(channelID: $channelID) {{ {FIELDS} }} }}"
    )
}

pub fn details_query(ids: &[String]) -> String {
    let fields = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            format!(
                "campaign_{i}: dropsCampaign(id: {}) {{ {FIELDS} }}",
                serde_json::to_string(id).unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("query AvailableDropsCampaigns {{ {fields} }}")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Campaign {
    pub id: String,
    pub name: String,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    reward_groups: Vec<Group>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    id: String,
    name: String,
    progress_criteria: Option<Criteria>,
    rewards: Vec<Reward>,
    #[serde(rename = "self")]
    progress: Option<Progress>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Criteria {
    requirement_type: String,
    is_auto_progressable: bool,
    requirements: Requirements,
    channels: Option<Vec<Id>>,
    categories: Option<Vec<Id>>,
    repeatable_config: Option<Repeats>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Requirements {
    minutes_watched: Option<u64>,
    subs: Option<u64>,
    turbo_subs: Option<u64>,
}

#[derive(Deserialize)]
struct Id {
    id: String,
    login: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repeats {
    repeatable_times: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reward {
    name: String,
    distribution_type: String,
    is_account_connected: Option<bool>,
    account_link_url: Option<String>,
    pool: Option<Pool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pool {
    selection_mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    status: String,
    has_preconditions_met: Option<bool>,
    current_minutes_watched: Option<u64>,
    current_subs: Option<u64>,
    current_turbo_subs: Option<u64>,
    grant_count: u64,
    next_eligible_at: Option<DateTime<Utc>>,
    current_window: Option<Window>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drop {
    pub campaign_id: String,
    pub group_id: String,
    pub campaign: String,
    pub reward: String,
    pub minutes: Option<u64>,
    pub required: u64,
    pub grants: Option<u64>,
    pub repeats: Option<u64>,
    pub status: DropStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DropStatus {
    Watching,
    Claimable,
    Claimed,
    Waiting(DateTime<Utc>),
    AwaitingReset,
    LinkRequired,
    Conditions,
    Unknown,
    Subscription,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    pub drop: Drop,
    pub channels: Vec<String>,
    pub categories: Vec<String>,
    pub ends_at: DateTime<Utc>,
    pub link_url: Option<String>,
}

impl CatalogEntry {
    pub fn detail(&self) -> String {
        let state = if self.drop.pending() {
            tr!("Eligible", "Можно фармить").into()
        } else {
            self.drop.label()
        };
        let mut text = format!(
            "{state}: {}\n{}\n{}; {} UTC",
            self.drop.campaign,
            self.drop.reward,
            self.drop.label(),
            self.ends_at.format("%Y-%m-%d %H:%M")
        );
        if let Some((total, grants)) = self.drop.repeats.zip(self.drop.grants) {
            text.push_str(&tf!(
                "; rewards {grants}/{total}",
                "; наград {grants}/{total}"
            ));
        }
        if self.drop.status == DropStatus::LinkRequired
            && let Some(url) = &self.link_url
        {
            text.push_str(&format!("\n{url}"));
        }
        text
    }
}

pub fn catalog(campaigns: Vec<Campaign>) -> Vec<CatalogEntry> {
    let now = Utc::now();
    let mut entries = vec![];
    for campaign in campaigns {
        for group in &campaign.reward_groups {
            let Some(drop) = campaign.drop(group, None, None, now, true) else {
                continue;
            };
            let criteria = group.progress_criteria.as_ref();
            entries.push(CatalogEntry {
                channels: criteria
                    .and_then(|c| c.channels.as_ref())
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.login.clone())
                    .collect(),
                categories: criteria
                    .and_then(|c| c.categories.as_ref())
                    .into_iter()
                    .flatten()
                    .map(|id| id.id.clone())
                    .collect(),
                link_url: group
                    .rewards
                    .iter()
                    .find_map(|r| r.account_link_url.clone()),
                ends_at: campaign.end_at,
                drop,
            });
        }
    }
    entries.sort_by(|a, b| {
        a.ends_at
            .cmp(&b.ends_at)
            .then_with(|| a.drop.campaign.cmp(&b.drop.campaign))
    });
    entries
}

impl Drop {
    pub fn pending(&self) -> bool {
        self.status == DropStatus::Watching
    }

    pub fn claim_id(&self, user_id: &str) -> Option<String> {
        // This composite ID is the Twitch web client's claim protocol.
        (self.status == DropStatus::Claimable)
            .then(|| format!("{user_id}#{}#{}", self.campaign_id, self.group_id))
    }

    pub fn label(&self) -> String {
        match self.status {
            DropStatus::Watching => tf!(
                "{}/{} min",
                "{}/{} мин",
                self.minutes.map_or_else(|| "?".into(), |m| m.to_string()),
                self.required
            ),
            DropStatus::Claimable => tr!("Ready to claim", "Можно забрать").into(),
            DropStatus::Claimed => tr!("Claimed", "Получено").into(),
            DropStatus::Waiting(_) => tr!("Waiting", "Ожидание").into(),
            DropStatus::AwaitingReset => tr!("Reset pending", "Ждём сброса").into(),
            DropStatus::LinkRequired => tr!("Link account", "Связать аккаунт").into(),
            DropStatus::Conditions => tr!("Other conditions", "Другие условия").into(),
            DropStatus::Unknown => tr!("Unknown status", "Статус неизвестен").into(),
            DropStatus::Subscription => tr!("Subscription required", "Нужна подписка").into(),
        }
    }

    pub fn detail(&self) -> String {
        let mut text = format!("{} / {}: {}", self.campaign, self.reward, self.label());
        if let Some((repeats, grants)) = self.repeats.zip(self.grants) {
            text.push_str(&tf!(
                "; rewards {}/{repeats}",
                "; наград {}/{repeats}",
                grants
            ));
        }
        if let DropStatus::Waiting(at) = self.status {
            text.push_str(&format!("; {} UTC", at.format("%Y-%m-%d %H:%M")));
        }
        if self.status == DropStatus::AwaitingReset {
            text.push_str(tr!(
                "; Twitch has not provided the next viewing time",
                "; Twitch не сообщил время следующего просмотра"
            ));
        }
        text
    }
}

pub fn parse(data: &Value, channel_id: &str, game_id: Option<&str>) -> Result<Vec<Drop>> {
    let value = data.get("channelDropCampaignsProgress").context(tr!(
        "Twitch did not return Drops progress",
        "Twitch не вернул прогресс Drops"
    ))?;
    let campaigns = parse_campaigns(value)?;
    Ok(campaigns
        .into_iter()
        .flat_map(|c| c.drops(channel_id, game_id, Utc::now()))
        .collect())
}

pub fn parse_campaigns(value: &Value) -> Result<Vec<Campaign>> {
    if value.is_null() {
        Ok(vec![])
    } else {
        serde_json::from_value(value.clone()).context(tr!(
            "Invalid Twitch Drops response",
            "Неверный ответ Twitch Drops"
        ))
    }
}

impl Campaign {
    fn drops(&self, channel_id: &str, game_id: Option<&str>, now: DateTime<Utc>) -> Vec<Drop> {
        self.reward_groups
            .iter()
            .filter_map(|group| self.drop(group, Some(channel_id), game_id, now, false))
            .collect()
    }

    fn drop(
        &self,
        group: &Group,
        channel_id: Option<&str>,
        game_id: Option<&str>,
        now: DateTime<Utc>,
        catalog: bool,
    ) -> Option<Drop> {
        if now < self.start_at || now >= self.end_at {
            return None;
        }
        let criteria = group.progress_criteria.as_ref()?;
        let required = criteria.requirements.minutes_watched.unwrap_or(0);
        if !catalog && (criteria.requirement_type != "WATCH" || required == 0) {
            return None;
        }
        let allowed = |ids: &Option<Vec<Id>>, target: Option<&str>| {
            ids.as_ref().is_none_or(|ids| {
                ids.is_empty() || ids.iter().any(|id| Some(id.id.as_str()) == target)
            })
        };
        if !catalog
            && (!allowed(&criteria.channels, channel_id) || !allowed(&criteria.categories, game_id))
        {
            return None;
        }
        let link_required = group.rewards.iter().any(|r| {
            !matches!(
                r.distribution_type.as_str(),
                "BADGE" | "EMOTE" | "CODE" | "POOL"
            ) && r.is_account_connected == Some(false)
        });
        let link_unknown = group.rewards.iter().any(|r| {
            !matches!(
                r.distribution_type.as_str(),
                "BADGE" | "EMOTE" | "CODE" | "POOL"
            ) && r.is_account_connected.is_none()
        });
        let choice_required = group
            .rewards
            .iter()
            .any(|r| r.pool.as_ref().is_some_and(|p| p.selection_mode != "CHEST"));
        let progress = group.progress.as_ref();
        let paid = criteria.requirement_type != "WATCH"
            || required == 0
            || criteria.requirements.subs.unwrap_or(0)
                > progress.and_then(|p| p.current_subs).unwrap_or(0)
            || criteria.requirements.turbo_subs.unwrap_or(0)
                > progress.and_then(|p| p.current_turbo_subs).unwrap_or(0);
        if paid && !catalog {
            return None;
        }
        let window_end = progress
            .and_then(|p| p.current_window.as_ref())
            .and_then(|w| w.expires_at);
        let current_reward_earned = progress.is_some_and(|p| {
            criteria.repeatable_config.is_some()
                && p.grant_count > 0
                && required > 0
                && p.current_minutes_watched
                    .is_some_and(|minutes| minutes >= required)
        });
        let status = match progress {
            Some(p) if p.has_preconditions_met == Some(false) => DropStatus::Conditions,
            Some(p) if p.status == "CLAIMABLE" && !choice_required && !paid => {
                DropStatus::Claimable
            }
            Some(p)
                if p.grant_count > 0
                    && criteria.repeatable_config.as_ref().is_none_or(|r| {
                        r.repeatable_times > 0 && p.grant_count >= r.repeatable_times
                    }) =>
            {
                DropStatus::Claimed
            }
            Some(p) if p.next_eligible_at.is_some_and(|at| at > now) => {
                DropStatus::Waiting(p.next_eligible_at.unwrap())
            }
            Some(p) if matches!(p.status.as_str(), "CLAIMED" | "FULFILLED") => DropStatus::Claimed,
            // Twitch can leave IN_PROGRESS and nextEligibleAt null after a daily grant.
            Some(_) if current_reward_earned && window_end.is_some_and(|at| at > now) => {
                DropStatus::Waiting(window_end.unwrap())
            }
            Some(_) if current_reward_earned && window_end.is_none() => DropStatus::AwaitingReset,
            // Counted watch time confirms progress even when the link flag disagrees.
            Some(p)
                if p.status == "IN_PROGRESS"
                    && p.current_minutes_watched.is_some_and(|minutes| minutes > 0)
                    && criteria.is_auto_progressable
                    && !choice_required
                    && !paid =>
            {
                DropStatus::Watching
            }
            _ if link_required => DropStatus::LinkRequired,
            _ if link_unknown => DropStatus::Unknown,
            _ if paid => DropStatus::Subscription,
            _ if choice_required || !criteria.is_auto_progressable => DropStatus::Conditions,
            Some(p) if p.status == "IN_PROGRESS" => DropStatus::Watching,
            None => DropStatus::Unknown,
            _ => DropStatus::Unknown,
        };
        Some(Drop {
            campaign_id: self.id.clone(),
            group_id: group.id.clone(),
            campaign: self.name.clone(),
            reward: if group.rewards.is_empty() {
                group.name.clone()
            } else {
                group
                    .rewards
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            minutes: progress.and_then(|p| p.current_minutes_watched),
            required,
            grants: progress.map(|p| p.grant_count),
            repeats: criteria
                .repeatable_config
                .as_ref()
                .map(|r| r.repeatable_times),
            status,
        })
    }
}

pub fn confirm_claim(data: &Value) -> Result<()> {
    let result = &data["claimDropRewards"];
    ensure!(
        result.get("error").is_none_or(Value::is_null)
            && matches!(
                result["status"].as_str(),
                Some("ELIGIBLE_FOR_ALL" | "DROP_INSTANCE_ALREADY_CLAIMED")
            ),
        "{}",
        tf!(
            "Twitch did not confirm the Drops claim",
            "Twitch не подтвердил получение Drops"
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn campaign() -> Value {
        json!({
            "id": "campaign", "name": "GTAVI Launch Celebration",
            "startAt": "2026-11-18T11:00:00Z", "endAt": "2026-12-01T07:59:59.999Z",
            "rewardGroups": [{
                "id": "group", "name": "GTA DVD Drops",
                "progressCriteria": {
                    "requirementType": "WATCH", "isAutoProgressable": true,
                    "requirements": {"minutesWatched": 60, "subs": null, "turboSubs": null},
                    "channels": null, "categories": [{"id": "gta"}],
                    "repeatableConfig": {"repeatableTimes": 6, "shouldCompleteAllRepeatsToEarn": false}
                },
                "rewards": [{"name": "GTA DVD Chest", "distributionType": "POOL",
                    "isAccountConnected": false, "pool": {"selectionMode": "CHEST"}}],
                "self": {"status": "IN_PROGRESS", "currentMinutesWatched": 12,
                    "grantCount": 3, "nextEligibleAt": null, "currentWindow": null}
            }]
        })
    }

    fn drops(value: Value) -> Vec<Drop> {
        drops_at(value, "2026-11-22T12:00:00Z".parse().unwrap())
    }

    fn drops_at(value: Value, now: DateTime<Utc>) -> Vec<Drop> {
        let campaign: Campaign = serde_json::from_value(value).unwrap();
        campaign.drops("channel", Some("gta"), now)
    }

    fn window_campaign(now: DateTime<Utc>) -> Value {
        let mut value = campaign();
        value["startAt"] = json!(now - chrono::Duration::days(1));
        value["endAt"] = json!(now + chrono::Duration::days(10));
        value
    }

    #[test]
    fn reward_campaign_uses_its_current_window_not_cumulative_minutes() {
        let drop = drops(campaign()).pop().unwrap();
        assert_eq!(drop.minutes, Some(12));
        assert_eq!(drop.required, 60);
        assert_eq!(drop.grants, Some(3));
        assert_eq!(drop.repeats, Some(6));
        assert!(drop.pending());
        assert!(drop.claim_id("123").is_none());
    }

    #[test]
    fn granted_daily_reward_waits_for_the_current_window_to_expire() {
        let now = Utc::now();
        let mut value = window_campaign(now);
        let reset = now + chrono::Duration::hours(4);
        value["rewardGroups"][0]["self"]["grantCount"] = json!(1);
        value["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(60);
        value["rewardGroups"][0]["self"]["currentWindow"] = json!({"expiresAt": reset});
        let drop = drops_at(value.clone(), now).pop().unwrap();
        assert_eq!(drop.status, DropStatus::Waiting(reset));
        assert!(!drop.pending());
        assert!(drop.claim_id("123").is_none());

        let campaign: Campaign = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            campaign
                .drop(
                    &campaign.reward_groups[0],
                    None,
                    None,
                    reset - chrono::Duration::seconds(1),
                    true
                )
                .unwrap()
                .status,
            DropStatus::Waiting(reset)
        );
        assert!(
            campaign
                .drop(&campaign.reward_groups[0], None, None, reset, true)
                .unwrap()
                .pending()
        );

        value["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(12);
        assert!(drops_at(value.clone(), now)[0].pending());
        value["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(60);
        value["rewardGroups"][0]["self"]["grantCount"] = json!(0);
        assert!(drops_at(value.clone(), now)[0].pending());
        value["rewardGroups"][0]["self"]["grantCount"] = json!(1);
        value["rewardGroups"][0]["self"]["currentWindow"] = Value::Null;
        assert_eq!(
            drops_at(value.clone(), now)[0].status,
            DropStatus::AwaitingReset
        );
        value["rewardGroups"][0]["self"]["nextEligibleAt"] = json!(reset);
        assert_eq!(
            drops_at(value.clone(), now)[0].status,
            DropStatus::Waiting(reset)
        );
        value["rewardGroups"][0]["self"]["status"] = json!("CLAIMABLE");
        assert_eq!(drops_at(value, now)[0].status, DropStatus::Claimable);
    }

    #[test]
    fn viewing_windows_are_independent_for_each_account_and_reward() {
        let now = Utc::now();
        let mut first_account = window_campaign(now);
        first_account["rewardGroups"][0]["self"]["grantCount"] = json!(1);
        first_account["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(60);
        let second_reward = first_account["rewardGroups"][0].clone();
        first_account["rewardGroups"]
            .as_array_mut()
            .unwrap()
            .push(second_reward);
        first_account["rewardGroups"][1]["id"] = json!("second-reward");
        first_account["rewardGroups"][0]["self"]["currentWindow"] =
            json!({"expiresAt": now + chrono::Duration::hours(4)});
        first_account["rewardGroups"][1]["self"]["currentWindow"] =
            json!({"expiresAt": now + chrono::Duration::hours(12)});
        let mut second_account = first_account.clone();
        second_account["rewardGroups"][0]["self"]["currentWindow"] =
            json!({"expiresAt": now + chrono::Duration::hours(48)});
        second_account["rewardGroups"][1]["self"]["currentWindow"] =
            json!({"expiresAt": now - chrono::Duration::seconds(1)});
        let first = drops_at(first_account, now);
        let second = drops_at(second_account, now);
        assert_eq!(
            first[0].status,
            DropStatus::Waiting(now + chrono::Duration::hours(4))
        );
        assert_eq!(
            first[1].status,
            DropStatus::Waiting(now + chrono::Duration::hours(12))
        );
        assert_eq!(
            second[0].status,
            DropStatus::Waiting(now + chrono::Duration::hours(48))
        );
        assert!(second[1].pending());
    }

    #[test]
    fn daily_window_completion_uses_each_rewards_watch_requirement() {
        let now = Utc::now();
        let reset = now + chrono::Duration::hours(8);
        for required in [15, 60, 90, 120] {
            let mut value = window_campaign(now);
            value["rewardGroups"][0]["progressCriteria"]["requirements"]["minutesWatched"] =
                json!(required);
            value["rewardGroups"][0]["self"]["grantCount"] = json!(1);
            value["rewardGroups"][0]["self"]["currentWindow"] = json!({"expiresAt": reset});
            value["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(required - 1);
            assert!(
                drops_at(value.clone(), now)[0].pending(),
                "Stopped before {required} minutes"
            );
            value["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(required);
            let drop = drops_at(value, now).pop().unwrap();
            assert_eq!(drop.required, required);
            assert_eq!(drop.status, DropStatus::Waiting(reset));
        }
    }

    #[test]
    fn confirmed_grants_complete_single_and_fully_earned_repeat_rewards() {
        let mut value = campaign();
        value["rewardGroups"][0]["self"]["grantCount"] = json!(6);
        assert_eq!(drops(value.clone())[0].status, DropStatus::Claimed);
        value["rewardGroups"][0]["progressCriteria"]["repeatableConfig"] = Value::Null;
        value["rewardGroups"][0]["self"]["grantCount"] = json!(1);
        assert_eq!(drops(value)[0].status, DropStatus::Claimed);
    }

    #[test]
    fn claims_require_server_status_even_above_the_minute_goal() {
        let mut value = campaign();
        value["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(675);
        assert!(drops(value.clone())[0].claim_id("123").is_none());
        value["rewardGroups"][0]["self"]["status"] = json!("CLAIMABLE");
        value["rewardGroups"][0]["self"]["nextEligibleAt"] = json!("2026-11-23T12:00:00Z");
        assert_eq!(
            drops(value)[0].claim_id("123").as_deref(),
            Some("123#campaign#group")
        );
        assert!(
            confirm_claim(&json!({"claimDropRewards": {"status": "ELIGIBLE_FOR_ALL"}})).is_ok()
        );
        for data in [
            json!({}),
            json!({"claimDropRewards": null}),
            json!({"claimDropRewards": {"status": "IN_PROGRESS"}}),
            json!({"claimDropRewards": {"status": "ELIGIBLE_FOR_ALL", "error": {"code": "FAILED"}}}),
        ] {
            assert!(confirm_claim(&data).is_err());
        }
    }

    #[test]
    fn paid_and_locked_rewards_do_not_become_watch_targets() {
        let mut value = campaign();
        value["rewardGroups"][0]["progressCriteria"]["requirements"]["subs"] = json!(1);
        assert!(drops(value).is_empty());
        let mut value = campaign();
        value["rewardGroups"][0]["progressCriteria"]["isAutoProgressable"] = json!(false);
        assert_eq!(drops(value)[0].status, DropStatus::Conditions);
        let mut value = campaign();
        value["rewardGroups"][0]["rewards"][0]["pool"]["selectionMode"] = json!("CHOICE");
        assert_eq!(drops(value)[0].status, DropStatus::Conditions);
    }

    #[test]
    fn account_connection_and_prerequisites_must_allow_viewing() {
        let mut value = campaign();
        value["rewardGroups"][0]["rewards"][0]["distributionType"] = json!("DIRECT_ENTITLEMENT");
        value["rewardGroups"][0]["rewards"][0]["pool"] = Value::Null;
        value["rewardGroups"][0]["self"]["currentMinutesWatched"] = Value::Null;
        assert_eq!(drops(value.clone())[0].status, DropStatus::LinkRequired);
        value["rewardGroups"][0]["rewards"][0]["isAccountConnected"] = Value::Null;
        assert_eq!(drops(value.clone())[0].status, DropStatus::Unknown);
        value["rewardGroups"][0]["rewards"][0]["isAccountConnected"] = json!(true);
        value["rewardGroups"][0]["self"]["hasPreconditionsMet"] = json!(false);
        assert_eq!(drops(value.clone())[0].status, DropStatus::Conditions);
        value["rewardGroups"][0]["self"]["hasPreconditionsMet"] = json!(true);
        assert!(drops(value)[0].pending());
    }

    #[test]
    fn counted_progress_and_claimable_status_override_a_stale_link_flag() {
        let mut value = campaign();
        value["rewardGroups"][0]["rewards"][0]["distributionType"] = json!("DIRECT_ENTITLEMENT");
        value["rewardGroups"][0]["rewards"][0]["pool"] = Value::Null;
        value["rewardGroups"][0]["rewards"][0]["isAccountConnected"] = json!(false);
        value["rewardGroups"][0]["self"]["currentMinutesWatched"] = json!(1);
        value["rewardGroups"][0]["self"]["hasPreconditionsMet"] = json!(true);
        assert!(drops(value.clone())[0].pending());
        assert!(drops(value.clone())[0].claim_id("123").is_none());
        value["rewardGroups"][0]["self"]["status"] = json!("CLAIMABLE");
        assert!(drops(value.clone())[0].claim_id("123").is_some());
        value["rewardGroups"][0]["self"]["hasPreconditionsMet"] = json!(false);
        assert_eq!(drops(value)[0].status, DropStatus::Conditions);
    }

    #[test]
    fn watch_rewards_with_paid_requirements_only_qualify_after_they_are_met() {
        let mut value = campaign();
        value["rewardGroups"][0]["progressCriteria"]["requirements"]["subs"] = json!(1);
        assert!(drops(value.clone()).is_empty());
        value["rewardGroups"][0]["self"]["currentSubs"] = json!(1);
        assert!(drops(value)[0].pending());
    }

    #[test]
    fn catalog_keeps_blocked_rewards_and_channel_restrictions() {
        let mut value = campaign();
        value["startAt"] = json!((Utc::now() - chrono::Duration::days(1)).to_rfc3339());
        value["endAt"] = json!((Utc::now() + chrono::Duration::days(1)).to_rfc3339());
        value["rewardGroups"][0]["progressCriteria"]["channels"] =
            json!([{"id": "510794436", "login": "whoplohoyparen"}]);
        value["rewardGroups"][0]["progressCriteria"]["requirements"]["subs"] = json!(1);
        let entries = catalog(vec![serde_json::from_value(value).unwrap()]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].drop.status, DropStatus::Subscription);
        assert_eq!(entries[0].channels, ["whoplohoyparen"]);
        assert_eq!(entries[0].categories, ["gta"]);
    }

    #[test]
    fn channel_and_category_restrictions_are_checked_at_group_level() {
        let mut value = campaign();
        value["rewardGroups"][0]["progressCriteria"]["categories"] = json!([{"id": "other"}]);
        assert!(drops(value).is_empty());
        let mut value = campaign();
        value["rewardGroups"][0]["progressCriteria"]["channels"] = json!([{"id": "other"}]);
        assert!(drops(value).is_empty());
        let mut value = campaign();
        value["rewardGroups"][0]["progressCriteria"]["categories"] = json!([]);
        assert_eq!(drops(value).len(), 1);
    }

    #[test]
    fn daily_reset_completed_and_missing_progress_are_distinct() {
        let mut value = campaign();
        value["rewardGroups"][0]["self"]["nextEligibleAt"] = json!("2026-11-23T12:00:00Z");
        assert!(matches!(drops(value)[0].status, DropStatus::Waiting(_)));
        let mut value = campaign();
        value["rewardGroups"][0]["self"]["status"] = json!("FULFILLED");
        assert_eq!(drops(value)[0].status, DropStatus::Claimed);
        let mut value = campaign();
        value["rewardGroups"][0]["self"]["currentMinutesWatched"] = Value::Null;
        assert_eq!(drops(value)[0].minutes, None);
        let mut value = campaign();
        value["rewardGroups"][0]["self"] = Value::Null;
        assert_eq!(drops(value)[0].status, DropStatus::Unknown);
        let mut value = campaign();
        value["endAt"] = json!("2026-11-21T12:00:00Z");
        assert!(drops(value).is_empty());
        assert!(parse(&json!({}), "channel", Some("gta")).is_err());
        assert!(
            parse(
                &json!({"channelDropCampaignsProgress": null}),
                "channel",
                Some("gta")
            )
            .unwrap()
            .is_empty()
        );
    }
}
