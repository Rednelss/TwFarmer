use crate::twitch::Channel;
use anyhow::{Context, Result};
use serde_json::{Value, json};

pub fn minute_watched_payload(channel: &Channel, user_id: &str) -> Result<Value> {
    let user_id: u64 = user_id
        .parse()
        .context("Twitch вернул нечисловой ID зрителя")?;
    Ok(json!([{"event":"minute-watched", "properties":{
        "channel_id":channel.id,
        "broadcast_id":channel.broadcast_id,
        "player":"site",
        "user_id":user_id,
        "live":true,
        "channel":channel.login
    }}]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> Channel {
        Channel {
            login: "new_channel".into(),
            id: "456".into(),
            broadcast_id: Some("789".into()),
        }
    }

    #[test]
    fn tracking_identifies_the_viewer_as_a_number_and_keeps_channel_ids() {
        let payload = minute_watched_payload(&channel(), "123").unwrap();
        assert_eq!(payload[0]["event"], "minute-watched");
        assert_eq!(payload[0]["properties"]["user_id"], 123);
        assert!(payload[0]["properties"]["user_id"].is_u64());
        assert_eq!(payload[0]["properties"]["channel_id"], "456");
        assert_eq!(payload[0]["properties"]["broadcast_id"], "789");
        assert_eq!(payload[0]["properties"]["channel"], "new_channel");
    }

    #[test]
    fn invalid_viewer_ids_are_rejected_before_telemetry_is_sent() {
        for id in ["", "not-an-id", "-1"] {
            assert!(minute_watched_payload(&channel(), id).is_err());
        }
    }
}
