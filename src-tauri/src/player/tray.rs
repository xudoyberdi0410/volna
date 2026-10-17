//! The tray: «Сейчас: …», play/pause, next, previous, show, quit.
//!
//! Linux: a StatusNotifierItem through `ksni`, so a left click shows (or
//! re-creates) the window and the menu stays on the right click
//! (decision §8.1). Windows: Tauri's tray icon, left click = show.

use std::future::Future;
use std::pin::Pin;

use tauri::AppHandle;
use volna_player::{NowPlaying, PlayerHandle, Status};

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
        tauri::async_runtime::spawn(f(p));
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

pub fn start(app: &AppHandle, player: Option<PlayerHandle>) {
    imp::start(app, player);
}

#[cfg(target_os = "linux")]
mod imp {
    use ksni::menu::{MenuItem, StandardItem};
    use ksni::{Icon, ToolTip, Tray, TrayMethods};
    use tauri::AppHandle;
    use volna_player::PlayerHandle;

    use super::{Labels, next, prev, spawn_cmd, toggle};
    use crate::player::window;

    struct VolnaTray {
        app: AppHandle,
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
            window::show_or_create(&self.app);
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
                    activate: Box::new(|t: &mut Self| window::show_or_create(&t.app)),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Выход".into(),
                    activate: Box::new(|t: &mut Self| window::quit(&t.app)),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    /// The window icon as ARGB32 (network order), as SNI wants it.
    fn icon(app: &AppHandle) -> Vec<Icon> {
        let Some(img) = app.default_window_icon() else {
            return vec![];
        };
        let mut data = img.rgba().to_vec();
        for px in data.as_chunks_mut::<4>().0 {
            px.rotate_right(1);
        }
        vec![Icon {
            width: img.width() as i32,
            height: img.height() as i32,
            data,
        }]
    }

    pub fn start(app: &AppHandle, player: Option<PlayerHandle>) {
        let tray = VolnaTray {
            app: app.clone(),
            labels: player
                .as_ref()
                .map(|p| Labels::of(&p.now()))
                .unwrap_or_default(),
            player: player.clone(),
            icon: icon(app),
        };
        tauri::async_runtime::spawn(async move {
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
}

#[cfg(windows)]
mod imp {
    use tauri::AppHandle;
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use volna_player::PlayerHandle;

    use super::{Labels, next, prev, spawn_cmd, toggle};
    use crate::player::window;

    pub fn start(app: &AppHandle, player: Option<PlayerHandle>) {
        if let Err(e) = build(app, player) {
            tracing::warn!("no tray: {e}");
        }
    }

    fn build(app: &AppHandle, player: Option<PlayerHandle>) -> tauri::Result<()> {
        let labels = player
            .as_ref()
            .map(|p| Labels::of(&p.now()))
            .unwrap_or_default();
        let now = MenuItem::with_id(app, "now", &labels.now, false, None::<&str>)?;
        let toggle_item = MenuItem::with_id(
            app,
            "toggle",
            labels.toggle(),
            labels.has_track,
            None::<&str>,
        )?;
        let next_item =
            MenuItem::with_id(app, "next", "Следующий", labels.has_track, None::<&str>)?;
        let prev_item =
            MenuItem::with_id(app, "prev", "Предыдущий", labels.has_track, None::<&str>)?;
        let show = MenuItem::with_id(app, "show", "Показать Volna", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
        let sep = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(
            app,
            &[
                &now,
                &toggle_item,
                &next_item,
                &prev_item,
                &sep,
                &show,
                &quit,
            ],
        )?;
        let menu_player = player.clone();
        let mut builder = TrayIconBuilder::with_id("volna")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .tooltip(labels.tooltip())
            .on_menu_event(move |app, ev| match ev.id().as_ref() {
                "toggle" => spawn_cmd(&menu_player, toggle),
                "next" => spawn_cmd(&menu_player, next),
                "prev" => spawn_cmd(&menu_player, prev),
                "show" => window::show_or_create(app),
                "quit" => window::quit(app),
                _ => {}
            })
            .on_tray_icon_event(|tray, ev| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = ev
                {
                    window::show_or_create(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        let tray = builder.build(app)?;
        let Some(player) = player else {
            return Ok(());
        };
        tauri::async_runtime::spawn(async move {
            let mut rx = player.watch_now();
            let mut last = Labels::of(&rx.borrow());
            while rx.changed().await.is_ok() {
                let labels = Labels::of(&rx.borrow_and_update());
                if labels == last {
                    continue;
                }
                let _ = now.set_text(&labels.now);
                let _ = toggle_item.set_text(labels.toggle());
                for item in [&toggle_item, &next_item, &prev_item] {
                    let _ = item.set_enabled(labels.has_track);
                }
                let _ = tray.set_tooltip(Some(labels.tooltip()));
                last = labels;
            }
        });
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    pub fn start(_: &tauri::AppHandle, _: Option<volna_player::PlayerHandle>) {}
}
