//! YouTube plugin for KnobDeck — live viewers and the Studio dashboard, through the YouTube Data
//! API with a read-only Google sign-in.

mod yt;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{load_env, open_url, Action, Plugin, Result, Value, Visual};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Each API read costs quota; a minute keeps a day of viewer polling far inside the free quota.
const VIEWERS_POLL: Duration = Duration::from_secs(60);

fn viewers_label() -> Result<String> {
    let Some(b) = yt::active_broadcast()? else { return Ok("Offline".into()) };
    let id = b["id"].as_str().unwrap_or("");
    let v = yt::get(&format!("/videos?part=liveStreamingDetails&id={id}"))?;
    // concurrentViewers is a decimal string.
    Ok(match v["items"][0]["liveStreamingDetails"]["concurrentViewers"].as_str() {
        Some(n) => format!("{n} watching"),
        None => "Live".into(),
    })
}

/// Channel statistics move slowly (YouTube rounds subscriber counts to three digits): one read
/// per channel every 10 minutes is ~144 quota units a day, shared by every key on that channel.
const STATS_POLL: Duration = Duration::from_secs(600);
static STATS: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);

/// The `channels` query for what the user typed: a handle (`@name`, with or without the @), a
/// channel id (`UC…`), a channel URL, or nothing for their own channel.
fn channel_query(input: &str) -> String {
    let t = input.trim().trim_end_matches('/');
    let t = t.split(['?', '#']).next().unwrap_or(t);
    let t = t.rsplit_once("youtube.com/").map_or(t, |(_, rest)| rest);
    if t.is_empty() {
        return "mine=true".into();
    }
    if let Some(id) = t.strip_prefix("channel/") {
        return format!("id={}", knobdeck_sdk::oauth::encode(id.split('/').next().unwrap_or(id)));
    }
    if let Some(name) = t.strip_prefix("user/") {
        return format!("forUsername={}", knobdeck_sdk::oauth::encode(name.split('/').next().unwrap_or(name)));
    }
    if t.len() == 24 && t.starts_with("UC") && !t.contains(['@', '/']) {
        return format!("id={t}");
    }
    let handle = t.split('/').next().unwrap_or(t).trim_start_matches('@');
    format!("forHandle={}", knobdeck_sdk::oauth::encode(&format!("@{handle}")))
}

/// The channel's `items[0]` with `snippet` and `statistics`, cached for `STATS_POLL`.
fn channel(input: &str) -> Result<Value> {
    let q = channel_query(input);
    if let Some((at, v)) = STATS.lock().unwrap().get_or_insert_with(HashMap::new).get(&q) {
        if at.elapsed() < STATS_POLL - Duration::from_secs(5) {
            return Ok(v.clone());
        }
    }
    let item = yt::get(&format!("/channels?part=snippet,statistics&{q}"))?["items"][0].clone();
    if item.is_null() {
        return Err(knobdeck_sdk::Error::msg(format!("No YouTube channel {}", if input.trim().is_empty() { "on this account" } else { input.trim() })));
    }
    STATS.lock().unwrap().get_or_insert_with(HashMap::new).insert(q, (Instant::now(), item.clone()));
    Ok(item)
}

/// 1234 → "1,234"; 12345 → "12.3K"; 4560000 → "4.56M".
fn compact(n: u64) -> String {
    let short = |v: f64, unit: &str| {
        let s = if v < 10.0 { format!("{v:.2}") } else if v < 100.0 { format!("{v:.1}") } else { format!("{v:.0}") };
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        format!("{s}{unit}")
    };
    match n {
        0..=9_999 => {
            let d = n.to_string();
            if d.len() > 3 { format!("{},{}", &d[..d.len() - 3], &d[d.len() - 3..]) } else { d }
        }
        10_000..=999_999 => short(n as f64 / 1e3, "K"),
        1_000_000..=999_999_999 => short(n as f64 / 1e6, "M"),
        _ => short(n as f64 / 1e9, "B"),
    }
}

fn stat_label(item: &Value, stat: &str) -> String {
    let s = &item["statistics"];
    // Counts are decimal strings.
    let n = |k: &str| s[k].as_str().and_then(|v| v.parse().ok()).unwrap_or(0u64);
    match stat {
        "views" => format!("{} views", compact(n("viewCount"))),
        "videos" => format!("{} videos", compact(n("videoCount"))),
        _ if s["hiddenSubscriberCount"] == true => "Subscribers hidden".into(),
        _ => format!("{} subscribers", compact(n("subscriberCount"))),
    }
}

fn main() {
    load_env();
    knobdeck_sdk::secrets::migrate_env(&["GOOGLE_CLIENT_SECRET"]);
    Plugin::new(&format!("youtube-{}", std::env::consts::OS), "YouTube")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("YouTube live viewers, channel stats and the Studio live dashboard (read-only sign-in).")
        .icon("app-icon.svg")
        .ack_timeout_ms(5000)
        .setup("youtube-setup", None)
        .setup_inputs(vec![
            input::text("client_id", "Your Google app's client ID (optional)").placeholder("leave empty to use KnobDeck's"),
            input::text("client_secret", "Your Google app's client secret (optional)").with("help", "Only for your own Google Cloud app (type Desktop, YouTube Data API enabled). Kept in this computer's credential store."),
        ])
        .action(
            Action::button("viewers", "Viewers")
                .icon("eye")
                .live(VIEWERS_POLL, |_| Ok(Visual::new().label(viewers_label().unwrap_or_else(|e| e.message().to_string()))))
                .on_press(|ctx| {
                    ctx.set_label(viewers_label()?);
                    Ok(())
                }),
        )
        // Write actions (need youtube.force-ssl, not requested for now for privacy; going live is
        // covered by the OBS plugin). Restore with the helpers in yt.rs and the scope in setup.
        //.action(Action::button("chat-message", "Chat Message").icon("message-square").inputs(vec![input::textarea("message", "Message").placeholder("Links: …")]).on_press(|ctx| {
        //    let message = ctx.input_str("message");
        //    if message.trim().is_empty() {
        //        return ctx.fail("Write the message first");
        //    }
        //    yt::chat(message.trim())
        //}))
        //.action(
        //    Action::button("play-ad", "Play Ad")
        //        .icon("megaphone")
        //        .inputs(vec![input::select("seconds", "Length", &[("30", "30 s"), ("60", "1 min"), ("90", "1 min 30 s"), ("120", "2 min")]).with("default", "30")])
        //        .on_press(|ctx| {
        //            let b = yt::live()?;
        //            let secs: i64 = ctx.input_str("seconds").parse().unwrap_or(30);
        //            yt::call("POST", &format!("/liveBroadcasts/cuepoint?id={}", b["id"].as_str().unwrap_or("")), Some(json!({"cueType": "cueTypeAd", "durationSecs": secs})))?;
        //            ctx.set_label(format!("Ad {secs}s"));
        //            Ok(())
        //        }),
        //)
        .action(
            Action::button("channel-stats", "Channel Stats")
                .icon("users")
                .inputs(vec![
                    input::text("channel", "Channel").placeholder("@handle or channel ID; empty for yours"),
                    input::select("stat", "Show", &[("subscribers", "Subscribers"), ("views", "Total views"), ("videos", "Videos")]).with("default", "subscribers"),
                ])
                .live(Duration::from_secs(60), |ic| Ok(Visual::new().label(match channel(&ic.input_str("channel")) {
                    Ok(item) => stat_label(&item, &ic.input_str("stat")),
                    Err(e) => e.message().to_string(),
                })))
                // Press opens the channel.
                .on_press(|ctx| {
                    let item = channel(&ctx.input_str("channel"))?;
                    open_url(&format!("https://www.youtube.com/channel/{}", item["id"].as_str().unwrap_or("")))
                }),
        )
        .action(Action::button("dashboard", "Live Dashboard").icon("layout-dashboard").on_press(|_| {
            let ch = yt::get("/channels?part=id&mine=true")?;
            let id = ch["items"][0]["id"].as_str().unwrap_or("");
            open_url(&format!("https://studio.youtube.com/channel/{id}/livestreaming"))
        }))
        //.action(
        //    Action::toggle("stream", "Go Live / End Stream")
        //        .icon("radio-tower")
        //        .state("offline", "Offline", "circle")
        //        .state("live", "Live", "circle-dot")
        //        .live(VIEWERS_POLL, |_| Ok(Visual::new().state(if yt::active_broadcast()?.is_some() { "live" } else { "offline" })))
        //        // Goes live with the next scheduled broadcast; ends the live one. YouTube needs the
        //        // encoder (OBS…) to be sending video before a broadcast can go live.
        //        .on_press(|ctx| {
        //            let (b, to) = match yt::active_broadcast()? {
        //                Some(b) => (b, "complete"),
        //                None => (yt::upcoming_broadcast()?.ok_or_else(|| knobdeck_sdk::Error::msg("No scheduled stream to go live with — create one in YouTube Studio"))?, "live"),
        //            };
        //            yt::call("POST", &format!("/liveBroadcasts/transition?broadcastStatus={to}&id={}&part=status", b["id"].as_str().unwrap_or("")), None)?;
        //            ctx.set_state(if to == "live" { "live" } else { "offline" });
        //            Ok(())
        //        }),
        //)
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    #[test]
    fn channel_input_forms() {
        assert_eq!(channel_query(""), "mine=true");
        assert_eq!(channel_query("@MrBeast"), "forHandle=%40MrBeast");
        assert_eq!(channel_query("mrbeast"), "forHandle=%40mrbeast");
        assert_eq!(channel_query("UCX6OQ3DkcsbYNE6H8uQQuVA"), "id=UCX6OQ3DkcsbYNE6H8uQQuVA");
        assert_eq!(channel_query("https://www.youtube.com/@MrBeast/videos"), "forHandle=%40MrBeast");
        assert_eq!(channel_query("https://www.youtube.com/channel/UCX6OQ3DkcsbYNE6H8uQQuVA/"), "id=UCX6OQ3DkcsbYNE6H8uQQuVA");
        assert_eq!(channel_query("youtube.com/@name?si=x"), "forHandle=%40name");
        assert_eq!(channel_query("https://youtube.com/user/oldname"), "forUsername=oldname");
    }

    #[test]
    fn counts_read_well() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(1234), "1,234");
        assert_eq!(compact(12_345), "12.3K");
        assert_eq!(compact(100_000), "100K");
        assert_eq!(compact(4_560_000), "4.56M");
        assert_eq!(compact(1_000_000), "1M");
        assert_eq!(compact(2_300_000_000), "2.3B");
        let item = json!({"statistics": {"subscriberCount": "4560000", "viewCount": "1234", "videoCount": "87", "hiddenSubscriberCount": false}});
        assert_eq!(stat_label(&item, "subscribers"), "4.56M subscribers");
        assert_eq!(stat_label(&item, "views"), "1,234 views");
        assert_eq!(stat_label(&item, "videos"), "87 videos");
        assert_eq!(stat_label(&json!({"statistics": {"hiddenSubscriberCount": true}}), "subscribers"), "Subscribers hidden");
    }
}
