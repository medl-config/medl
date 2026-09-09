use medl::inputs::Inputs;
use medl::loader::FsLoader;
use medl::Medl;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

fn entry() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/app.medl")
}

fn inputs(variant: &str, platform: &str, secrets: &[(&str, &str)]) -> Inputs {
    Inputs::new()
        .with_ctx("variant", variant)
        .with_ctx("platform", platform)
        .with_env(HashMap::<String, String>::new())
        .with_secret(
            secrets
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
        )
}

const SECRETS: &[(&str, &str)] = &[
    ("RELAY_APP_ID", "relay-123"),
    ("ANALYTICS_FALLBACK_KEY", "fallback-key"),
];

fn resolve(variant: &str, platform: &str) -> serde_json::Value {
    let mut medl = Medl::new(Box::new(FsLoader));
    let out = medl
        .resolve(&entry(), &inputs(variant, platform, SECRETS))
        .unwrap_or_else(|e| panic!("{}", e.render(&medl.sources)));
    let warnings: Vec<String> = out
        .warnings
        .iter()
        .map(|w| w.render(&medl.sources))
        .collect();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    out.value.to_json()
}

#[test]
fn store_mobile() {
    assert_eq!(
        resolve("store", "mobile"),
        json!({
            "app": {
                "name": "Pulse",
                "bundle_id": "com.example.pulse",
                "version": "1.4.0",
                "features": ["telemetry", "crash_reporting", "enterprise_sso", "device_management"],
                "build": {"min_os": "17.0", "optimization": "release"},
                "networking": {"timeout_seconds": 30, "retry_count": 3},
                "variant_name": "Pulse Store",
                "sku": "PLS-STORE",
                "enterprise": {"enabled": true, "mdm_provider": "meridian", "license_seats": 500},
                "session": {"provider": "airlink", "max_participants": 8, "label": "AIRLINK"},
                "platform_features": ["push_notifications", "health_sync"],
                "platform_supported": true,
                "build_id": "PLS-STORE-mobile-1_4_0",
                "display_name": "Pulse Store",
                "compliance_profile": "enterprise",
                "identifiers": {"module_class": "PulseStoreSession", "log_tag": "pulse_store"},
                "debug_summary": "features: telemetry, crash_reporting, enterprise_sso, device_management (4 total)",
                "analytics_key": "fallback-key"
            }
        })
    );
}

#[test]
fn expo_xr() {
    assert_eq!(
        resolve("expo", "xr"),
        json!({
            "app": {
                "name": "Pulse",
                "bundle_id": "com.example.pulse",
                "version": "1.4.0",
                "features": ["telemetry", "session_recording"],
                "build": {"min_os": "2.0", "optimization": "release"},
                "networking": {"timeout_seconds": 15, "retry_count": 3},
                "variant_name": "Pulse Expo",
                "sku": "PLS-EXPO",
                "enterprise": {"enabled": false},
                "session": {
                    "provider": "relay",
                    "app_id": "relay-123",
                    "region": "eu",
                    "max_participants": 40,
                    "spatial_audio": true,
                    "label": "RELAY"
                },
                "platform_features": ["spatial_personas", "shared_space"],
                "platform_supported": true,
                "build_id": "PLS-EXPO-xr-1_4_0",
                "display_name": "Pulse Expo for XR",
                "compliance_profile": "consumer",
                "identifiers": {"module_class": "PulseExpoSession", "log_tag": "pulse_expo"},
                "debug_summary": "features: telemetry, session_recording (2 total)",
                "analytics_key": "fallback-key"
            }
        })
    );
}

#[test]
fn store_xr_and_expo_mobile_spot_checks() {
    let v = resolve("store", "xr");
    assert_eq!(v["app"]["display_name"], "Pulse Store for XR");
    assert_eq!(v["app"]["build"]["min_os"], "2.0");
    assert_eq!(v["app"]["session"]["spatial_audio"], true);
    assert_eq!(v["app"]["networking"]["timeout_seconds"], 30);
    let v = resolve("expo", "mobile");
    assert_eq!(v["app"]["display_name"], "Pulse Expo");
    assert_eq!(v["app"]["build_id"], "PLS-EXPO-mobile-1_4_0");
    assert_eq!(v["app"]["networking"]["timeout_seconds"], 15);
    assert_eq!(v["app"]["session"].get("spatial_audio"), None);
}

/// `platform_supported` is declared before `display_name` in examples/app.medl, and neither
/// depends on a config value, so resolve order is declaration order and its error() is what
/// surfaces — not the uncovered `when` subject in `display_name`.
#[test]
fn unsupported_platform_errors_through_the_error_builtin() {
    let mut medl = Medl::new(Box::new(FsLoader));
    let err = medl
        .resolve(&entry(), &inputs("store", "web", SECRETS))
        .unwrap_err();
    assert_eq!(err.message, "unsupported platform: web");
}

#[test]
fn expo_without_the_relay_secret_errors() {
    let mut medl = Medl::new(Box::new(FsLoader));
    let err = medl
        .resolve(
            &entry(),
            &inputs("expo", "mobile", &[("ANALYTICS_FALLBACK_KEY", "k")]),
        )
        .unwrap_err();
    assert_eq!(
        err.message,
        "secret `RELAY_APP_ID` is not available and no fallback was given"
    );
}
