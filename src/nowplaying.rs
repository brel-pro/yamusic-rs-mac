//! Карточка «Сейчас играет» в Пункте управления macOS и медиаклавиши. Состояние плеер
//! отдаёт сам, команды приходят в поток интерфейса. Без пакета .app система подписывает
//! карточку именем файла, `yamusic-rs`.

use crate::api::Track;
use crate::config::log;
use crate::player::{Player, Shared};
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};
use std::time::Duration;

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

pub enum Event {
    PlayPause,
    Play,
    Pause,
    Next,
    Prev,
    Seek(f32),
}

/// Поднять центр управления. Ошибка здесь не останавливает плеер.
pub fn spawn(player: Player, on_event: impl Fn(Event) + Send + 'static) {
    let started = std::thread::Builder::new().name("nowplaying".into()).spawn(move || {
        let mut controls = match MediaControls::new(PlatformConfig {
            display_name: env!("APP_NAME"),
            dbus_name: "yamusic-rs",
            hwnd: None,
        }) {
            Ok(c) => c,
            Err(e) => {
                log(format!("Сейчас играет: {e}"));
                return;
            }
        };
        if let Err(e) = controls.attach(move |e| {
            if let Some(e) = map(e) {
                on_event(e);
            }
        }) {
            log(format!("Сейчас играет: {e}"));
            return;
        }
        follow(&mut controls, &player);
    });
    if let Err(e) = started {
        log(format!("Сейчас играет: поток не запустился: {e}"));
    }
}

fn map(e: MediaControlEvent) -> Option<Event> {
    Some(match e {
        MediaControlEvent::Toggle => Event::PlayPause,
        MediaControlEvent::Play => Event::Play,
        MediaControlEvent::Pause | MediaControlEvent::Stop => Event::Pause,
        MediaControlEvent::Next => Event::Next,
        MediaControlEvent::Previous => Event::Prev,
        MediaControlEvent::SetPosition(p) => Event::Seek(p.0.as_secs_f32()),
        _ => return None,
    })
}

fn follow(controls: &mut MediaControls, player: &Player) {
    let mut track: Option<String> = None;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let s = player.state();
        let id = s.track.as_ref().map(|t| t.id.clone());
        if id != track {
            let title = s.track.as_ref().map(title);
            let artist = s.track.as_ref().map(|t| t.artists.clone()).filter(|a| !a.is_empty());
            let album = s.track.as_ref().map(|t| t.album_title.clone()).filter(|a| !a.is_empty());
            let cover = s.track.as_ref().and_then(|t| t.cover_url(400));
            let secs = s.track.as_ref().map(|t| if s.duration > 0.0 { s.duration } else { t.duration_ms as f32 / 1000.0 });
            let _ = controls.set_metadata(MediaMetadata {
                title: title.as_deref(),
                artist: artist.as_deref(),
                album: album.as_deref(),
                cover_url: cover.as_deref(),
                duration: secs.filter(|d| *d > 0.0).map(|d| Duration::from_secs_f32(d)),
            });
            track = id;
        }
        let _ = controls.set_playback(playback(&s));
    }
}

fn playback(s: &Shared) -> MediaPlayback {
    let progress = Some(MediaPosition(Duration::from_secs_f32(s.position.max(0.0))));
    match (&s.track, s.playing) {
        (None, _) => MediaPlayback::Stopped,
        (Some(_), true) => MediaPlayback::Playing { progress },
        (Some(_), false) => MediaPlayback::Paused { progress },
    }
}

fn title(t: &Track) -> String {
    if t.version.is_empty() { t.title.clone() } else { format!("{} ({})", t.title, t.version) }
}
