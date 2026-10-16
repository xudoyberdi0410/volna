//! Hits the real music.youtube.com and api.ipify.org over the direct route.
//! Run with `cargo test -p volna-net --test live_health -- --ignored --nocapture`.

use volna_net::{NetManager, NetOptions};

#[tokio::test]
#[ignore = "needs internet access"]
async fn direct_health_check() {
    let net = NetManager::new(NetOptions::default()).unwrap();
    let report = net.health_check().await;
    println!("{report:#?}");
    assert!(report.ok, "{report:?}");
    assert!(report.exit_ip.is_some());
}
