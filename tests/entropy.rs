//! Integration test: verify distinct RNG outputs across N forked VMs.
//!
//! Requires: KVM access + pre-built `workdir-python/` template.
//! Run with: `sudo cargo test --test entropy -- --ignored --nocapture`

use std::collections::HashSet;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Deserialize)]
struct ExecResponse {
    stdout: String,
    #[allow(dead_code)]
    exit_code: i32,
}

fn start_server(port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_zeroboot"))
        .args(["serve", "workdir-python", &port.to_string()])
        .env("ZEROBOOT_API_KEYS_FILE", "/dev/null")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start zeroboot server")
}

fn wait_healthy(base: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() > deadline {
            panic!("server did not become healthy within 10s");
        }
        if ureq::get(&format!("{base}/v1/health")).call().is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn exec_code(base: &str, code: &str) -> String {
    let resp: ExecResponse = ureq::post(&format!("{base}/v1/exec"))
        .send_json(&serde_json::json!({ "code": code }))
        .expect("request failed")
        .body_mut()
        .read_json()
        .expect("invalid JSON response");

    assert_eq!(resp.exit_code, 0, "non-zero exit: stdout={}", resp.stdout);

    resp.stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string()
}

#[test]
#[ignore] // requires KVM + pre-built workdir-python/ template
fn entropy_uniqueness() {
    // Fixed port: port-0 discovery isn't practical through a subprocess.
    // This test requires sudo + KVM, so port conflicts are unlikely.
    let port = 18080;
    let mut server = start_server(port);
    let base = format!("http://127.0.0.1:{port}");

    wait_healthy(&base);

    let n = 10;
    let tests = [
        ("os.urandom (kernel CRNG)", "print(os.urandom(16).hex())"),
        ("random.random (stdlib)", "print(random.random())"),
        ("numpy.random (numpy)", "print(numpy.random.random())"),
    ];

    for (name, code) in tests {
        let mut values = HashSet::new();
        for _ in 0..n {
            let stdout = exec_code(&base, code);
            assert!(!stdout.is_empty(), "{name}: empty stdout");
            values.insert(stdout);
        }
        assert_eq!(
            values.len(),
            n,
            "{name}: expected {n} unique values, got {} — values: {values:?}",
            values.len()
        );
    }

    server.kill().ok();
    server.wait().ok();
}
