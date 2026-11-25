//! Provider usage limits for the title bar (`claude_usage.rs`, `codex_usage.rs`).
//!
//! Off by default, per provider (`preferences.usageAccess`), as in the web build: reading usage
//! means reading the CLI's stored login (`~/.claude/.credentials.json`, `~/.codex/auth.json`) and
//! sending that token to the provider that issued it. Nothing runs for a provider that is off.
//!
//! One background thread polls every five minutes (immediately when a provider is switched on).
//! A failed refresh keeps the last reading on screen, marked stale; a missing login clears it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

const POLL: Duration = Duration::from_secs(5 * 60);
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    pub percent: f64,
    /// Epoch ms; None when the provider did not say.
    pub resets_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct ClaudeUsage {
    pub five_hour: Window,
    pub seven_day: Window,
    pub seven_day_opus: Window,
    /// Weekly limits scoped to one model, e.g. ("Fable", window).
    pub model_limits: Vec<(String, Window)>,
}

#[derive(Clone, Debug, Default)]
pub struct CodexUsage {
    /// The short (5 h) window; zero-length when the plan has none.
    pub primary: Window,
    pub primary_minutes: u64,
    /// The weekly window.
    pub secondary: Window,
    pub plan: String,
    pub rate_limited: bool,
    pub reset_credits: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Reading<T> {
    pub value: Option<T>,
    /// The last refresh failed; `value` is older.
    pub stale: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct Shared {
    claude: Reading<ClaudeUsage>,
    codex: Reading<CodexUsage>,
}

pub struct UsageMonitor {
    shared: Arc<Mutex<Shared>>,
    claude_on: Arc<AtomicBool>,
    codex_on: Arc<AtomicBool>,
    /// Ask the thread to refresh now (a provider was just switched on).
    poke: Arc<AtomicBool>,
}

impl UsageMonitor {
    pub fn start(ctx: egui::Context) -> Self {
        let monitor = Self {
            shared: Arc::default(),
            claude_on: Arc::default(),
            codex_on: Arc::default(),
            poke: Arc::default(),
        };
        let shared = Arc::clone(&monitor.shared);
        let claude_on = Arc::clone(&monitor.claude_on);
        let codex_on = Arc::clone(&monitor.codex_on);
        let poke = Arc::clone(&monitor.poke);
        let _ = std::thread::Builder::new()
            .name("usage-poll".into())
            .spawn(move || {
                let mut claude_at: Option<Instant> = None;
                let mut codex_at: Option<Instant> = None;
                loop {
                    let forced = poke.swap(false, Ordering::Relaxed);
                    let due =
                        |at: Option<Instant>| forced || at.is_none_or(|t| t.elapsed() >= POLL);
                    if claude_on.load(Ordering::Relaxed) {
                        if due(claude_at) {
                            claude_at = Some(Instant::now());
                            let result = fetch_claude();
                            if let Err(e) = &result {
                                eprintln!("[usage] claude: {e}");
                            }
                            update(&mut shared.lock().unwrap().claude, result, "no_token");
                            ctx.request_repaint();
                        }
                    } else if claude_at.take().is_some() {
                        shared.lock().unwrap().claude = Reading::default();
                        ctx.request_repaint();
                    }
                    if codex_on.load(Ordering::Relaxed) {
                        if due(codex_at) {
                            codex_at = Some(Instant::now());
                            let result = fetch_codex();
                            if let Err(e) = &result {
                                eprintln!("[usage] codex: {e}");
                            }
                            update(&mut shared.lock().unwrap().codex, result, "no_oauth_token");
                            ctx.request_repaint();
                        }
                    } else if codex_at.take().is_some() {
                        shared.lock().unwrap().codex = Reading::default();
                        ctx.request_repaint();
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            });
        monitor
    }

    /// Mirrors `preferences.usageAccess`; switching a provider on refreshes it right away.
    pub fn set_access(&self, claude: bool, codex: bool) {
        let was_claude = self.claude_on.swap(claude, Ordering::Relaxed);
        let was_codex = self.codex_on.swap(codex, Ordering::Relaxed);
        if (claude && !was_claude) || (codex && !was_codex) {
            self.poke.store(true, Ordering::Relaxed);
        }
    }

    /// Refreshes the providers that are on now (a click on a pill).
    pub fn refresh(&self) {
        self.poke.store(true, Ordering::Relaxed);
    }

    pub fn claude(&self) -> Reading<ClaudeUsage> {
        self.shared.lock().unwrap().claude.clone()
    }

    pub fn codex(&self) -> Reading<CodexUsage> {
        self.shared.lock().unwrap().codex.clone()
    }
}

fn update<T>(reading: &mut Reading<T>, result: Result<T, String>, missing_login: &str) {
    match result {
        Ok(value) => {
            *reading = Reading {
                value: Some(value),
                stale: false,
                error: None,
            }
        }
        Err(error) => {
            if error.contains(missing_login) {
                reading.value = None;
            }
            reading.stale = reading.value.is_some();
            reading.error = Some(error);
        }
    }
}

/// `usagePillColor`: green under 50 %, amber under 80 %, red above.
pub fn level(percent: f64) -> u8 {
    if percent >= 80.0 {
        2
    } else if percent >= 50.0 {
        1
    } else {
        0
    }
}

/// `formatResetTime`: "2h 14m", "38m", or "…" once due.
pub fn reset_in(resets_at_ms: Option<u64>, now_ms: u64) -> Option<String> {
    let at = resets_at_ms?;
    if at <= now_ms {
        return Some("…".into());
    }
    let minutes = (at - now_ms) / 60_000;
    let (h, m) = (minutes / 60, minutes % 60);
    Some(if h >= 24 {
        format!("{}d {}h", h / 24, h % 24)
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    })
}

// ----------------------------------------------------------------- HTTP

fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .provider(ureq::tls::TlsProvider::NativeTls)
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into()
    })
}

fn get_json(request: ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Result<Value, String> {
    let mut response = request.call().map_err(|e| format!("request failed: {e}"))?;
    response
        .body_mut()
        .read_json::<Value>()
        .map_err(|e| format!("json parse: {e}"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

// ----------------------------------------------------------------- Claude

/// The OAuth token Claude Code stored: `CLAUDE_OAUTH_TOKEN`, else `~/.claude/.credentials.json`.
fn claude_token() -> Option<String> {
    if let Ok(token) = std::env::var("CLAUDE_OAUTH_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }
    let path = home()?.join(".claude").join(".credentials.json");
    // Claude Code rewrites the file while refreshing; a torn read is retried.
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(80));
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(token) = json
            .pointer("/claudeAiOauth/accessToken")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            return Some(token.to_string());
        }
    }
    None
}

fn fetch_claude() -> Result<ClaudeUsage, String> {
    let token = claude_token().ok_or("no_token")?;
    let body = get_json(
        agent()
            .get(CLAUDE_USAGE_URL)
            .header("Authorization", &format!("Bearer {token}"))
            .header("anthropic-beta", "oauth-2025-04-20"),
    )?;
    Ok(parse_claude(&body))
}

fn parse_claude(body: &Value) -> ClaudeUsage {
    let window = |key: &str| -> Window {
        let Some(obj) = body.get(key).filter(|v| !v.is_null()) else {
            return Window::default();
        };
        Window {
            percent: obj
                .get("utilization")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            resets_at_ms: obj
                .get("resets_at")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms),
        }
    };
    // Weekly limits scoped to a model (`weekly_scoped` with a model and no surface).
    let mut model_limits: Vec<(String, Window)> = body
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|l| l.get("kind").and_then(Value::as_str) == Some("weekly_scoped"))
        .filter(|l| l.pointer("/scope/surface").is_none_or(Value::is_null))
        .filter_map(|l| {
            let model = l
                .pointer("/scope/model/display_name")?
                .as_str()?
                .to_string();
            Some((
                model,
                Window {
                    percent: l.get("percent").and_then(Value::as_f64).unwrap_or(0.0),
                    resets_at_ms: l
                        .get("resets_at")
                        .and_then(Value::as_str)
                        .and_then(parse_rfc3339_ms),
                },
            ))
        })
        .collect();
    // Opus has its own row: when the legacy window is missing, it comes from `limits[]`.
    let opus_at = model_limits
        .iter()
        .position(|(m, _)| m.eq_ignore_ascii_case("opus"));
    let opus = opus_at.map(|i| model_limits.remove(i).1);
    let seven_day_opus = match opus {
        Some(window) if body.get("seven_day_opus").is_none_or(Value::is_null) => window,
        _ => window("seven_day_opus"),
    };
    ClaudeUsage {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
        seven_day_opus,
        model_limits,
    }
}

// ----------------------------------------------------------------- Codex

fn codex_auth_path() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join("auth.json"))
        .or_else(|| Some(home()?.join(".codex").join("auth.json")))
}

/// Reads the quota with the token the Codex CLI stored. Read-only on purpose: refreshing here
/// would rotate the refresh token under the CLI.
fn fetch_codex() -> Result<CodexUsage, String> {
    let path = codex_auth_path().ok_or("no_home")?;
    let raw = std::fs::read(&path).map_err(|e| format!("auth read failed: {e}"))?;
    let auth: Value =
        serde_json::from_slice(&raw).map_err(|e| format!("auth parse failed: {e}"))?;
    let tokens = auth.get("tokens").ok_or("no_oauth_token")?;
    let text = |key: &str| {
        tokens
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let token = text("access_token").ok_or("no_oauth_token")?;
    let mut request = agent()
        .get(CODEX_USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("User-Agent", concat!("Alethe/", env!("CARGO_PKG_VERSION")));
    if let Some(account) = text("account_id") {
        request = request.header("ChatGPT-Account-Id", &account);
    }
    parse_codex(&get_json(request)?)
}

fn parse_codex(body: &Value) -> Result<CodexUsage, String> {
    let limits = body
        .get("rate_limit")
        .filter(|v| !v.is_null())
        .ok_or("no rate limits")?;
    let window = |key: &str| -> (Window, u64) {
        let field = |name: &str| {
            limits
                .get(key)
                .and_then(|w| w.get(name))
                .and_then(Value::as_f64)
        };
        (
            Window {
                percent: field("used_percent").unwrap_or(0.0),
                resets_at_ms: field("reset_at").map(|s| (s * 1000.0) as u64),
            },
            field("limit_window_seconds").map_or(0, |s| (s / 60.0) as u64),
        )
    };
    let (mut primary, mut secondary) = (window("primary_window"), window("secondary_window"));
    // Some plans send only a weekly window, as `primary`: keep `secondary` the long one.
    const DAY_MINUTES: u64 = 24 * 60;
    if primary.1 > DAY_MINUTES && secondary.1 <= DAY_MINUTES {
        std::mem::swap(&mut primary, &mut secondary);
    }
    Ok(CodexUsage {
        primary: primary.0,
        primary_minutes: primary.1,
        secondary: secondary.0,
        plan: body
            .get("plan_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        rate_limited: limits
            .get("limit_reached")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        reset_credits: body
            .pointer("/rate_limit_reset_credits/available_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

// ----------------------------------------------------------------- time

/// `2026-09-24T02:59:59.123+00:00` → epoch ms. Enough RFC 3339 for API timestamps.
fn parse_rfc3339_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let num = |s: &str| s.parse::<i64>().ok();
    let (date, rest) = text.split_once(['T', ' '])?;
    let mut d = date.splitn(3, '-');
    let (y, mo, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let (time, offset_secs) = if let Some(t) = rest.strip_suffix(['Z', 'z']) {
        (t, 0)
    } else if let Some(i) = rest.rfind(['+', '-']) {
        let (t, off) = rest.split_at(i);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let (oh, om) = off[1..]
            .split_once(':')
            .unwrap_or((&off[1..3.min(off.len())], "0"));
        (t, sign * (num(oh)? * 3600 + num(om)? * 60))
    } else {
        (rest, 0)
    };
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut t = hms.splitn(3, ':');
    let (h, mi, s) = (
        num(t.next()?)?,
        num(t.next()?)?,
        num(t.next().unwrap_or("0"))?,
    );
    let millis = format!("{frac:0<3}").get(..3).and_then(num).unwrap_or(0);
    // Days from civil (Howard Hinnant).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + s - offset_secs;
    u64::try_from(secs * 1000 + millis).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-09-24T02:59:59Z"),
            Some(1_790_218_799_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-09-24T05:59:59.5+03:00"),
            Some(1_790_218_799_500)
        );
    }

    #[test]
    fn claude_model_limits_and_opus_fallback() {
        let usage = parse_claude(&json!({
            "five_hour": { "utilization": 15.0, "resets_at": "2026-09-24T02:59:59Z" },
            "seven_day": { "utilization": 30.0, "resets_at": "2026-09-24T01:59:59Z" },
            "seven_day_opus": null,
            "limits": [
                { "kind": "weekly_all", "percent": 30, "scope": null },
                { "kind": "weekly_scoped", "percent": 25, "resets_at": "2026-09-24T01:59:59Z",
                  "scope": { "model": { "display_name": "Fable" }, "surface": null } },
                { "kind": "weekly_scoped", "percent": 40, "resets_at": "2026-09-24T01:59:59Z",
                  "scope": { "model": { "display_name": "Opus" }, "surface": null } },
                { "kind": "weekly_scoped", "percent": 70,
                  "scope": { "model": { "display_name": "Fable" }, "surface": { "id": "cli" } } }
            ]
        }));
        assert_eq!(usage.five_hour.percent, 15.0);
        assert_eq!(usage.seven_day_opus.percent, 40.0);
        assert_eq!(usage.model_limits.len(), 1);
        assert_eq!(usage.model_limits[0].0, "Fable");
        assert_eq!(usage.model_limits[0].1.percent, 25.0);
    }

    #[test]
    fn codex_windows_and_weekly_only_plans() {
        let usage = parse_codex(&json!({
            "plan_type": "plus",
            "rate_limit": {
                "limit_reached": true,
                "primary_window": { "used_percent": 12, "limit_window_seconds": 18_000, "reset_at": 1_700_003_600 },
                "secondary_window": { "used_percent": 44.5, "limit_window_seconds": 604_800, "reset_at": 1_700_475_600 }
            },
            "rate_limit_reset_credits": { "available_count": 2 }
        }))
        .unwrap();
        assert_eq!(usage.primary.percent, 12.0);
        assert_eq!(usage.primary_minutes, 300);
        assert_eq!(usage.secondary.resets_at_ms, Some(1_700_475_600_000));
        assert!(usage.rate_limited);
        assert_eq!(usage.reset_credits, 2);

        let weekly_only = parse_codex(&json!({
            "rate_limit": { "primary_window": { "used_percent": 60, "limit_window_seconds": 604_800 } }
        }))
        .unwrap();
        assert_eq!(weekly_only.secondary.percent, 60.0);
        assert_eq!(weekly_only.primary_minutes, 0);
    }

    #[test]
    fn reset_labels() {
        assert_eq!(
            reset_in(Some(3_600_000 * 2 + 14 * 60_000), 0).as_deref(),
            Some("2h 14m")
        );
        assert_eq!(reset_in(Some(38 * 60_000), 0).as_deref(), Some("38m"));
        assert_eq!(reset_in(Some(50 * 3_600_000), 0).as_deref(), Some("2d 2h"));
        assert_eq!(reset_in(None, 0), None);
    }
}
