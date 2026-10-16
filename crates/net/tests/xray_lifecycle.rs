//! Tests that run a real local xray (`xray` on PATH or `$VOLNA_XRAY`). They
//! never leave 127.0.0.1. Run with `cargo test -p volna-net -- --ignored`.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use volna_net::vless::SocksInbound;
use volna_net::xray::{LogBuffer, XrayProcess, find_xray, free_local_port};
use volna_net::{NetError, NetManager, NetOptions, ProxyConfig, VlessLink};

const UUID: &str = "92cc34c0-f305-4528-9473-28bf84c6862b";
const TIMEOUT: Duration = Duration::from_secs(10);

/// SOCKS in, straight out.
fn socks_to_freedom(port: u16) -> Value {
    json!({
        "log": { "loglevel": "warning" },
        "inbounds": [{
            "listen": "127.0.0.1", "port": port, "protocol": "socks",
            "settings": { "auth": "noauth" }
        }],
        "outbounds": [{ "protocol": "freedom" }]
    })
}

/// A plain VLESS server on localhost, so the client side can be tested
/// end-to-end without any real server.
fn vless_server(port: u16) -> Value {
    json!({
        "log": { "loglevel": "warning" },
        "inbounds": [{
            "listen": "127.0.0.1", "port": port, "protocol": "vless",
            "settings": { "clients": [{ "id": UUID }], "decryption": "none" },
            "streamSettings": { "network": "tcp", "security": "none" }
        }],
        "outbounds": [{ "protocol": "freedom" }]
    })
}

async fn port_open(port: u16) -> bool {
    TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .is_ok()
}

async fn wait_closed(port: u16) -> bool {
    for _ in 0..100 {
        if !port_open(port).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Answers every request with `hello`.
async fn hello_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await;
            });
        }
    });
    port
}

#[tokio::test]
#[ignore = "needs an xray binary"]
async fn spawn_ready_and_kill_on_drop() {
    let binary = find_xray(None).expect("xray on PATH");
    let port = free_local_port().unwrap();
    let logs = LogBuffer::default();

    let proc = XrayProcess::spawn(&binary, &socks_to_freedom(port), port, &logs, TIMEOUT)
        .await
        .unwrap();
    assert!(proc.is_running());
    assert!(proc.pid().is_some());
    assert!(port_open(port).await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        logs.lines().iter().any(|l| l.contains("started")),
        "log: {:?}",
        logs.lines()
    );

    drop(proc);
    assert!(wait_closed(port).await, "xray still listening after drop");
}

#[tokio::test]
#[ignore = "needs an xray binary"]
async fn broken_config_reports_exit_with_log() {
    let binary = find_xray(None).expect("xray on PATH");
    let port = free_local_port().unwrap();
    let logs = LogBuffer::default();
    let bad = json!({ "inbounds": [{ "protocol": "no-such-protocol", "port": port }] });

    let err = XrayProcess::spawn(&binary, &bad, port, &logs, TIMEOUT)
        .await
        .unwrap_err();
    let NetError::XrayExited { log, .. } = err else {
        panic!("expected XrayExited, got {err:?}");
    };
    assert!(!log.is_empty());
}

#[tokio::test]
#[ignore = "needs an xray binary"]
async fn generated_configs_pass_xray_test() {
    let binary = find_xray(None).expect("xray on PATH");
    let links = [
        "?type=xhttp&security=reality&pbk=wigF4NuMt2kUCb5_gX1rosrHLsVAjNkWx6JhM9ntJ_s\
         &sid=41e126e9ee78dfbb&sni=www.microsoft.com&fp=chrome&path=%2Fx&mode=auto",
        "?type=tcp&security=reality&pbk=wigF4NuMt2kUCb5_gX1rosrHLsVAjNkWx6JhM9ntJ_s\
         &sid=41e1&sni=a.example&flow=xtls-rprx-vision",
        "?type=ws&security=tls&path=%2Fws&host=a.example&sni=a.example&alpn=h2,http%2F1.1",
        "?type=grpc&security=tls&serviceName=svc&mode=multi",
        "?type=httpupgrade&security=none&path=%2Fup",
        "?type=xhttp&security=tls&mode=packet-up&extra=%7B%22xPaddingBytes%22%3A%22100-1000%22%7D",
        "?type=tcp&headerType=http&security=none",
    ];
    for query in links {
        let link = VlessLink::parse(&format!("vless://{UUID}@203.0.113.1:443{query}")).unwrap();
        let config = link.xray_config(&SocksInbound {
            port: 1080,
            user: "u".into(),
            pass: "p".into(),
        });
        let mut child = tokio::process::Command::new(&binary)
            .args(["run", "-test", "-format", "json", "-c", "stdin:"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(&serde_json::to_vec(&config).unwrap())
            .await
            .unwrap();
        drop(stdin);
        let out = child.wait_with_output().await.unwrap();
        assert!(
            out.status.success(),
            "xray rejected config for {query}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// NetManager in vless mode against a local VLESS server: the request goes
/// app → reqwest → socks5h → xray client → VLESS → xray server → hello server.
#[tokio::test]
#[ignore = "needs an xray binary"]
async fn net_manager_vless_end_to_end() {
    let binary = find_xray(None).expect("xray on PATH");
    let server_port = free_local_port().unwrap();
    let server_logs = LogBuffer::default();
    let _server = XrayProcess::spawn(
        &binary,
        &vless_server(server_port),
        server_port,
        &server_logs,
        TIMEOUT,
    )
    .await
    .unwrap();
    let hello = hello_server().await;

    let net = NetManager::new(NetOptions::default()).unwrap();
    let link = format!("vless://{UUID}@127.0.0.1:{server_port}?type=tcp&security=none#local");
    net.apply(ProxyConfig::Vless(link.clone())).await.unwrap();

    let proxy = net.proxy_url().unwrap();
    assert!(proxy.starts_with("socks5h://volna:"), "{proxy}");
    let status = net.status();
    assert_eq!(status.mode, "vless");
    assert_eq!(status.xray_running, Some(true));
    assert!(!status.proxy.unwrap().contains(&proxy[18..30]));
    let client_port: u16 = proxy.rsplit(':').next().unwrap().parse().unwrap();

    let body = net
        .client()
        .get(format!("http://127.0.0.1:{hello}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "hello");

    // The inbound needs the generated credentials.
    let anon = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("socks5h://127.0.0.1:{client_port}")).unwrap())
        .build()
        .unwrap();
    assert!(
        anon.get(format!("http://127.0.0.1:{hello}/"))
            .send()
            .await
            .is_err()
    );

    // Re-applying starts a fresh xray on a new port and stops the old one.
    net.apply(ProxyConfig::Vless(link)).await.unwrap();
    let new_port: u16 = net
        .proxy_url()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(new_port, client_port);
    assert!(wait_closed(client_port).await);

    net.shutdown().await;
    assert_eq!(net.config(), ProxyConfig::Direct);
    assert!(wait_closed(new_port).await);
    assert!(!net.xray_logs().is_empty());
}
