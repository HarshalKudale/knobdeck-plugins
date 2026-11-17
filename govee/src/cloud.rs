//! Govee's cloud API (developer.govee.com, "router" API) for devices without LAN Control. Needs the
//! user's own API key (Govee Home ▸ Profile ▸ Settings ▸ Apply for API Key). The key allows
//! 10,000 requests a day, so cloud devices are read seldom and changed only on a press.

use knobdeck_sdk::{json, Error, Result, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BASE: &str = "https://openapi.api.govee.com/router/api/v1";

#[derive(Clone, Debug, PartialEq)]
pub struct CloudDevice {
    pub id: String,
    pub sku: String,
    pub name: String,
}

fn request_id() -> String {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos().to_string()).unwrap_or_default()
}

fn err(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Status(401, _) => Error::msg("Govee rejected the API key: run setup again"),
        ureq::Error::Status(429, _) => Error::msg("Govee's daily request limit is used up; try again tomorrow"),
        ureq::Error::Status(code, r) => {
            let msg = r.into_json::<Value>().ok().and_then(|v| v["message"].as_str().or(v["msg"].as_str()).map(String::from)).unwrap_or_default();
            Error::msg(format!("Govee answered {code} {msg}").trim().to_string())
        }
        e => Error::msg(format!("Govee's server did not answer ({e})")),
    }
}

fn post(key: &str, path: &str, payload: Value) -> Result<Value> {
    ureq::post(&format!("{BASE}{path}"))
        .set("Govee-API-Key", key)
        .timeout(Duration::from_secs(8))
        .send_json(json!({"requestId": request_id(), "payload": payload}))
        .map_err(err)?
        .into_json()
        .map_err(|e| Error::msg(format!("Govee sent an invalid reply: {e}")))
}

pub fn devices(key: &str) -> Result<Vec<CloudDevice>> {
    let v: Value = ureq::get(&format!("{BASE}/user/devices")).set("Govee-API-Key", key).timeout(Duration::from_secs(8)).call().map_err(err)?.into_json().map_err(|e| Error::msg(format!("Govee sent an invalid reply: {e}")))?;
    Ok(v["data"]
        .as_array()
        .map(|a| a.iter().map(|d| CloudDevice { id: d["device"].as_str().unwrap_or("").into(), sku: d["sku"].as_str().unwrap_or("").into(), name: d["deviceName"].as_str().unwrap_or("").into() }).collect())
        .unwrap_or_default())
}

pub fn control(key: &str, d: &CloudDevice, kind: &str, instance: &str, value: Value) -> Result<()> {
    post(key, "/device/control", json!({"sku": d.sku, "device": d.id, "capability": {"type": kind, "instance": instance, "value": value}}))?;
    Ok(())
}

/// The device's current capability values as `{instance: value}` (`powerSwitch`, `brightness`…).
pub fn state(key: &str, d: &CloudDevice) -> Result<Value> {
    let v = post(key, "/device/state", json!({"sku": d.sku, "device": d.id}))?;
    let mut out = serde_json::Map::new();
    for c in v["payload"]["capabilities"].as_array().into_iter().flatten() {
        if let Some(i) = c["instance"].as_str() {
            out.insert(i.into(), c["state"]["value"].clone());
        }
    }
    Ok(Value::Object(out))
}

/// The device's light scenes: (name, value to send back).
pub fn scenes(key: &str, d: &CloudDevice) -> Result<Vec<(String, Value)>> {
    let v = post(key, "/device/scenes", json!({"sku": d.sku, "device": d.id}))?;
    Ok(v["payload"]["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|c| c["parameters"]["options"].as_array().cloned().unwrap_or_default())
        .filter_map(|o| Some((o["name"].as_str()?.to_string(), o["value"].clone())))
        .collect())
}

pub fn rgb_int(r: u8, g: u8, b: u8) -> i64 {
    ((r as i64) << 16) | ((g as i64) << 8) | b as i64
}

#[cfg(test)]
mod tests {
    #[test]
    fn colors() {
        assert_eq!(super::rgb_int(255, 136, 0), 0xFF8800);
    }
}
