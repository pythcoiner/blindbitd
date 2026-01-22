use blindbitd::{BlindbitD, Conf, Features};
use serde::Deserialize;
use std::{thread, time::Duration};

#[derive(Debug, Deserialize)]
struct InfoResponse {
    network: String,
    height: u32,
    tweaks_only: bool,
    tweaks_full_basic: bool,
    tweaks_full_with_dust_filter: bool,
    tweaks_cut_through_with_dust_filter: bool,
}

fn new_blindbitd_instance() -> BlindbitD {
    let blindbitd = BlindbitD::new().unwrap();
    println!("BlindbitD running at {}:{}", blindbitd.addr, blindbitd.port);
    blindbitd
}

fn dump_logs(bbd: &mut BlindbitD) {
    while let Ok(log) = bbd.logs.try_recv() {
        println!("{log}");
    }
}

#[derive(Debug, Deserialize)]
struct BlockHeightResponse {
    block_height: u32,
}

fn wait_for_sync(url: &str, expected_height: u32) {
    let height_url = format!("{}/block-height", url);
    let timeout = std::time::Instant::now() + Duration::from_secs(30);

    loop {
        if std::time::Instant::now() > timeout {
            panic!("Timeout waiting for oracle to sync to height {}", expected_height);
        }

        if let Ok(resp) = ureq::get(&height_url).call() {
            if let Ok(body) = resp.into_string() {
                if let Ok(height_resp) = serde_json::from_str::<BlockHeightResponse>(&body) {
                    println!("Oracle height: {}", height_resp.block_height);
                    if height_resp.block_height >= expected_height {
                        return;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

#[test]
fn simple_blindbitd() {
    let mut bbd = new_blindbitd_instance();
    let mut node = bbd.bitcoin().unwrap();
    let bitcoind = &mut node.client;
    let address = bitcoind.new_address().unwrap();
    // Generate 100 blocks
    bitcoind.generate_to_address(100, &address).unwrap();
}

/// Test config parsing with default features (DustFilterCutThrough)
/// Dumps logs to verify how the oracle parsed the config values
#[test]
fn test_config_parsing_default() {
    let mut bbd = BlindbitD::new().unwrap();
    println!("=== BlindbitD with default config (DustFilterCutThrough) ===");
    println!("URL: {}", bbd.url());

    // Give it a moment to produce logs
    thread::sleep(Duration::from_millis(500));

    println!("\n=== Logs ===");
    dump_logs(&mut bbd);

    // Read the generated config file to see what was written
    let config_path = bbd.workdir().join("blindbit.toml");
    let config_content = std::fs::read_to_string(&config_path).unwrap();
    println!("\n=== Generated config file ===");
    println!("{}", config_content);
}

/// Test config parsing with FullBasic features
/// This should enable /tweak-index and disable /tweaks
#[test]
fn test_config_parsing_full_basic() {
    let conf = Conf::with_features(Features::FullBasic);
    let mut bbd = BlindbitD::with_conf(&conf).unwrap();
    println!("=== BlindbitD with FullBasic config ===");
    println!("URL: {}", bbd.url());

    // Give it a moment to produce logs
    thread::sleep(Duration::from_millis(500));

    println!("\n=== Logs ===");
    dump_logs(&mut bbd);

    // Read the generated config file
    let config_path = bbd.workdir().join("blindbit.toml");
    let config_content = std::fs::read_to_string(&config_path).unwrap();
    println!("\n=== Generated config file ===");
    println!("{}", config_content);
}

/// Test /info endpoint with default config (DustFilterCutThrough)
/// Verifies that the oracle correctly reports its feature flags
#[test]
fn test_info_endpoint_default() {
    let mut bbd = BlindbitD::new().unwrap();
    let mut node = bbd.bitcoin().unwrap();
    let bitcoind = &mut node.client;

    // Generate some blocks so the oracle has data to index
    let address = bitcoind.new_address().unwrap();
    bitcoind.generate_to_address(10, &address).unwrap();

    let base_url = bbd.url();

    // Wait for oracle to sync to height 10
    wait_for_sync(&base_url, 10);

    let url = format!("{}/info", base_url);

    println!("=== Calling {} ===", url);
    let result = ureq::get(&url).call();

    // Dump logs to see what happened
    thread::sleep(Duration::from_millis(100));
    println!("\n=== Logs after /info request ===");
    dump_logs(&mut bbd);

    let body = match result {
        Ok(resp) => resp.into_string().unwrap(),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_else(|_| "no body".to_string());
            panic!("HTTP {}: {}", code, body);
        }
        Err(e) => panic!("Request error: {:?}", e),
    };

    let response: InfoResponse = serde_json::from_str(&body).unwrap();

    println!("=== /info response (default config) ===");
    println!("{:#?}", response);

    // With default DustFilterCutThrough config:
    // - tweaks_cut_through_with_dust_filter should be true
    // - all others should be false
    assert!(!response.tweaks_only, "tweaks_only should be false");
    assert!(!response.tweaks_full_basic, "tweaks_full_basic should be false");
    assert!(
        !response.tweaks_full_with_dust_filter,
        "tweaks_full_with_dust_filter should be false"
    );
    assert!(
        response.tweaks_cut_through_with_dust_filter,
        "tweaks_cut_through_with_dust_filter should be true"
    );
}

/// Test /info endpoint with FullBasic config
/// Verifies that the oracle correctly reports its feature flags
#[test]
fn test_info_endpoint_full_basic() {
    let conf = Conf::with_features(Features::FullBasic);
    let mut bbd = BlindbitD::with_conf(&conf).unwrap();
    let mut node = bbd.bitcoin().unwrap();
    let bitcoind = &mut node.client;

    // Generate some blocks so the oracle has data to index
    let address = bitcoind.new_address().unwrap();
    bitcoind.generate_to_address(10, &address).unwrap();

    let base_url = bbd.url();

    // Wait for oracle to sync to height 10
    wait_for_sync(&base_url, 10);

    let url = format!("{}/info", base_url);

    println!("=== Calling {} ===", url);
    let result = ureq::get(&url).call();

    // Dump logs to see what happened
    thread::sleep(Duration::from_millis(100));
    println!("\n=== Logs after /info request ===");
    dump_logs(&mut bbd);

    let body = match result {
        Ok(resp) => resp.into_string().unwrap(),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_else(|_| "no body".to_string());
            panic!("HTTP {}: {}", code, body);
        }
        Err(e) => panic!("Request error: {:?}", e),
    };

    let response: InfoResponse = serde_json::from_str(&body).unwrap();

    println!("=== /info response (FullBasic config) ===");
    println!("{:#?}", response);

    // With FullBasic config:
    // - tweaks_full_basic should be true
    // - all others should be false
    assert!(!response.tweaks_only, "tweaks_only should be false");
    assert!(response.tweaks_full_basic, "tweaks_full_basic should be true");
    assert!(
        !response.tweaks_full_with_dust_filter,
        "tweaks_full_with_dust_filter should be false"
    );
    assert!(
        !response.tweaks_cut_through_with_dust_filter,
        "tweaks_cut_through_with_dust_filter should be false"
    );
}

// =============================================================================
// Endpoint behavior tests
//
// KEY FINDING: Both /tweaks and /tweak-index always return 200 with empty array
// regardless of config. The difference is only visible when there are actual
// taproot transactions - only the configured endpoint will have data populated.
//
// Client behavior: Check /info to determine which endpoint to use, then use
// the appropriate one based on feature flags.
// =============================================================================

/// Helper to make a GET request and return (status_code, body)
fn get_endpoint(url: &str) -> (u16, String) {
    match ureq::get(url).call() {
        Ok(resp) => (200, resp.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(code, resp)) => {
            (code, resp.into_string().unwrap_or_else(|_| "no body".to_string()))
        }
        Err(e) => panic!("Request error: {:?}", e),
    }
}

/// Test endpoint availability with DustFilterCutThrough config (default)
///
/// Both endpoints return 200, but only /tweaks will have data when there are
/// taproot transactions (the oracle indexes tweaks with cut-through compression).
#[test]
fn test_endpoints_with_cutthrough_config() {
    // DustFilterCutThrough is the default
    let mut bbd = BlindbitD::new().unwrap();
    let mut node = bbd.bitcoin().unwrap();
    let bitcoind = &mut node.client;

    let address = bitcoind.new_address().unwrap();
    bitcoind.generate_to_address(10, &address).unwrap();

    let base_url = bbd.url();
    wait_for_sync(&base_url, 10);

    // Both endpoints return 200 (empty arrays when no taproot txs)
    let (status, body) = get_endpoint(&format!("{}/tweaks/5", base_url));
    println!("/tweaks/5 response: status={}, body={}", status, body);
    assert_eq!(status, 200);
    let tweaks: Vec<String> = serde_json::from_str(&body).expect("Should parse as array");
    println!("Tweaks: {:?}", tweaks);

    let (status, body) = get_endpoint(&format!("{}/tweak-index/5", base_url));
    println!("/tweak-index/5 response: status={}, body={}", status, body);
    assert_eq!(status, 200);
    let tweak_index: Vec<String> = serde_json::from_str(&body).expect("Should parse as array");
    println!("Tweak index: {:?}", tweak_index);

    // With DustFilterCutThrough:
    // - /tweaks is the correct endpoint (has data when taproot txs exist)
    // - /tweak-index returns empty (index not built)
    // Note: Both are empty here because coinbase txs don't have taproot inputs
}

/// Test endpoint availability with FullBasic config
///
/// Both endpoints return 200, but only /tweak-index will have data when there
/// are taproot transactions (the oracle indexes full tweak index without cut-through).
#[test]
fn test_endpoints_with_full_basic_config() {
    let conf = Conf::with_features(Features::FullBasic);
    let mut bbd = BlindbitD::with_conf(&conf).unwrap();
    let mut node = bbd.bitcoin().unwrap();
    let bitcoind = &mut node.client;

    let address = bitcoind.new_address().unwrap();
    bitcoind.generate_to_address(10, &address).unwrap();

    let base_url = bbd.url();
    wait_for_sync(&base_url, 10);

    // Both endpoints return 200 (empty arrays when no taproot txs)
    let (status, body) = get_endpoint(&format!("{}/tweaks/5", base_url));
    println!("/tweaks/5 response: status={}, body={}", status, body);
    assert_eq!(status, 200);
    let tweaks: Vec<String> = serde_json::from_str(&body).expect("Should parse as array");
    println!("Tweaks: {:?}", tweaks);

    let (status, body) = get_endpoint(&format!("{}/tweak-index/5", base_url));
    println!("/tweak-index/5 response: status={}, body={}", status, body);
    assert_eq!(status, 200);
    let tweak_index: Vec<String> = serde_json::from_str(&body).expect("Should parse as array");
    println!("Tweak index: {:?}", tweak_index);

    // With FullBasic:
    // - /tweak-index is the correct endpoint (has data when taproot txs exist)
    // - /tweaks returns empty (cut-through index not built)
    // Note: Both are empty here because coinbase txs don't have taproot inputs
}
