//! The tray: «Сейчас: …», play/pause, next, previous, show, quit.
//!
//! Linux: a StatusNotifierItem through `ksni` (pure D-Bus), so a left click
//! shows (or starts) the window and the menu stays on the right click
//! (decision §8.1). Windows has no tray in the daemon yet (§9).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use volna_player::{NowPlaying, PlayerHandle, Status};

use crate::daemon::Daemon;

/// Menu labels for the current state.
#[derive(Debug, Clone, PartialEq, Default)]
struct Labels {
    now: String,
    playing: bool,
    has_track: bool,
}

impl Labels {
    fn of(n: &NowPlaying) -> Self {
        let playing = matches!(n.state.status, Status::Playing | Status::Loading);
        match &n.track {
            Some(t) => Self {
                now: format!("Сейчас: {}", t.display()),
                playing,
                has_track: true,
            },
            None => Self {
                now: "Ничего не играет".into(),
                playing: false,
                has_track: false,
            },
        }
    }

    fn toggle(&self) -> &'static str {
        if self.playing {
            "Пауза"
        } else {
            "Играть"
        }
    }

    fn tooltip(&self) -> String {
        if self.has_track {
            self.now.trim_start_matches("Сейчас: ").to_owned()
        } else {
            "Volna".into()
        }
    }
}

type Action = fn(PlayerHandle) -> Pin<Box<dyn Future<Output = ()> + Send>>;

fn spawn_cmd(player: &Option<PlayerHandle>, f: Action) {
    if let Some(p) = player.clone() {
        tokio::spawn(f(p));
    }
}

fn toggle(p: PlayerHandle) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let _ = p.toggle().await;
    })
}

fn next(p: PlayerHandle) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let _ = p.next().await;
    })
}

fn prev(p: PlayerHandle) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let _ = p.prev().await;
    })
}

pub fn start(daemon: Arc<Daemon>, player: Option<PlayerHandle>) {
    imp::start(daemon, player);
}

#[cfg(target_os = "linux")]
mod imp {
    use std::sync::Arc;

    use ksni::menu::{MenuItem, StandardItem};
    use ksni::{Icon, ToolTip, Tray, TrayMethods};
    use volna_player::PlayerHandle;

    use super::{Labels, next, prev, spawn_cmd, toggle};
    use crate::daemon::Daemon;

    struct VolnaTray {
        daemon: Arc<Daemon>,
        player: Option<PlayerHandle>,
        labels: Labels,
        icon: Vec<Icon>,
    }

    impl Tray for VolnaTray {
        fn id(&self) -> String {
            "volna".into()
        }

        fn title(&self) -> String {
            "Volna".into()
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            self.icon.clone()
        }

        fn tool_tip(&self) -> ToolTip {
            ToolTip {
                title: "Volna".into(),
                description: self.labels.tooltip(),
                ..Default::default()
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.daemon.show_window();
        }

        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            spawn_cmd(&self.player, toggle);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let has = self.labels.has_track;
            vec![
                StandardItem {
                    label: self.labels.now.replace('_', "__"),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: self.labels.toggle().into(),
                    enabled: has,
                    activate: Box::new(|t: &mut Self| spawn_cmd(&t.player, toggle)),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Следующий".into(),
                    enabled: has,
                    activate: Box::new(|t: &mut Self| spawn_cmd(&t.player, next)),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Предыдущий".into(),
                    enabled: has,
                    activate: Box::new(|t: &mut Self| spawn_cmd(&t.player, prev)),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "Показать Volna".into(),
                    activate: Box::new(|t: &mut Self| t.daemon.show_window()),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Выход".into(),
                    activate: Box::new(|t: &mut Self| t.daemon.quit()),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    /// The app icon as ARGB32 (network order), as SNI wants it.
    fn icons() -> Vec<Icon> {
        let pngs: [&[u8]; 2] = [
            include_bytes!("../../../../packaging/icons/32x32.png"),
            include_bytes!("../../../../packaging/icons/128x128.png"),
        ];
        pngs.iter().filter_map(|b| decode(b)).collect()
    }

    fn decode(bytes: &[u8]) -> Option<Icon> {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
        let mut reader = dec.read_info().ok()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).ok()?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return None;
        }
        let mut data = buf[..info.buffer_size()].to_vec();
        for px in data.as_chunks_mut::<4>().0 {
            px.rotate_right(1);
        }
        Some(Icon {
            width: info.width as i32,
            height: info.height as i32,
            data,
        })
    }

    pub fn start(daemon: Arc<Daemon>, player: Option<PlayerHandle>) {
        let tray = VolnaTray {
            daemon,
            labels: player
                .as_ref()
                .map(|p| Labels::of(&p.now()))
                .unwrap_or_default(),
            player: player.clone(),
            icon: icons(),
        };
        tokio::spawn(async move {
            let handle = match tray.assume_sni_available(true).spawn().await {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!("no tray: {e}");
                    return;
                }
            };
            let Some(player) = player else {
                return;
            };
            let mut now = player.watch_now();
            let mut last = Labels::of(&now.borrow());
            while now.changed().await.is_ok() {
                let labels = Labels::of(&now.borrow_and_update());
                if labels != last {
                    last = labels.clone();
                    handle
                        .update(move |t: &mut VolnaTray| t.labels = labels)
                        .await;
                }
            }
        });
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn icons_decode() {
            let icons = super::icons();
            assert_eq!(icons.len(), 2);
            assert_eq!(icons[0].width, 32);
            assert_eq!(icons[0].data.len(), 32 * 32 * 4);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    pub fn start(_: std::sync::Arc<crate::daemon::Daemon>, _: Option<volna_player::PlayerHandle>) {
        tracing::info!("no tray on this platform yet; a second launch shows the window");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let l = Labels::of(&NowPlaying::default());
        assert!(!l.has_track);
        assert_eq!(l.toggle(), "Играть");
        assert_eq!(l.tooltip(), "Volna");
    }
}
