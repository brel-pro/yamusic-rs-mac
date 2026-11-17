//! macOS: окно через winit, пути ~/Library, один экземпляр через unix-сокет,
//! диалоги через osascript, автозапуск через LaunchAgent. Трея нет, поэтому
//! закрытие окна закрывает приложение. Карточка «Сейчас играет» — в nowplaying.rs.

use slint::winit_030::{winit, WinitWindowAccessor};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

const APP_ID: &str = "yamusic-rs";

pub fn created(w: &slint::Window) -> bool {
    w.with_winit_window(|_| ()).is_some()
}

/// Значок в Dock и системная шапка: прозрачный заголовок, светофор, скругление.
pub fn prepare(w: &slint::Window) {
    if let Ok(img) = image::load_from_memory(include_bytes!("../../packaging/icons/256.png")) {
        let img = img.to_rgba8();
        let (iw, ih) = img.dimensions();
        if let Ok(icon) = winit::window::Icon::from_rgba(img.into_raw(), iw, ih) {
            w.with_winit_window(|w| w.set_window_icon(Some(icon)));
        }
    }
    w.with_winit_window(|win| {
        use winit::platform::macos::WindowExtMacOS;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        win.set_has_shadow(true);
        let Ok(handle) = win.window_handle() else { return };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
        traffic_lights(h.ns_view.as_ptr());
    });
}

/// Рамка остаётся, но заголовок не виден: контент на всю высоту, кнопки системы слева.
/// Зелёная разворачивает окно на экран, а не уводит его на отдельное пространство:
/// полноэкранный режим системы сбрасывает эту рамку и окно пропадает.
fn traffic_lights(view: *mut std::ffi::c_void) {
    const FULL_SIZE: usize = 1 << 15;
    const TITLE_HIDDEN: isize = 1;
    const SEPARATOR_NONE: isize = 1;
    const FULLSCREEN_PRIMARY: usize = 1 << 7;
    const FULLSCREEN_AUXILIARY: usize = 1 << 8;
    const FULLSCREEN_NONE: usize = 1 << 9;
    let window = msg_ptr(view, "window");
    if window.is_null() {
        return;
    }
    let mask = msg_usize(window, "styleMask");
    msg_set_usize(window, "setStyleMask:", mask | FULL_SIZE);
    msg_set_bool(window, "setTitlebarAppearsTransparent:", true);
    msg_set_isize(window, "setTitleVisibility:", TITLE_HIDDEN);
    msg_set_isize(window, "setTitlebarSeparatorStyle:", SEPARATOR_NONE);
    let behavior = msg_usize(window, "collectionBehavior");
    let behavior = behavior & !(FULLSCREEN_PRIMARY | FULLSCREEN_AUXILIARY) | FULLSCREEN_NONE;
    msg_set_usize(window, "setCollectionBehavior:", behavior);
    // 0 закрыть, 1 свернуть, 2 развернуть. Смена маски иногда их прячет.
    for kind in 0..3isize {
        let button = msg_ptr_isize(window, "standardWindowButton:", kind);
        if button.is_null() {
            continue;
        }
        msg_set_bool(button, "setHidden:", false);
        msg_set_bool(button, "setEnabled:", true);
    }
}

unsafe extern "C" {
    fn sel_registerName(name: *const i8) -> *mut u8;
    fn objc_msgSend();
}

fn sel(name: &str) -> *mut u8 {
    let c = std::ffi::CString::new(name).expect("sel");
    unsafe { sel_registerName(c.as_ptr()) }
}

fn msg_ptr(obj: *mut std::ffi::c_void, name: &str) -> *mut std::ffi::c_void {
    let f: unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8) -> *mut std::ffi::c_void =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, sel(name)) }
}

fn msg_ptr_isize(obj: *mut std::ffi::c_void, name: &str, val: isize) -> *mut std::ffi::c_void {
    let f: unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8, isize) -> *mut std::ffi::c_void =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, sel(name), val) }
}

fn msg_usize(obj: *mut std::ffi::c_void, name: &str) -> usize {
    let f: unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8) -> usize =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, sel(name)) }
}

fn msg_set_usize(obj: *mut std::ffi::c_void, name: &str, val: usize) {
    let f: unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8, usize) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, sel(name), val) }
}

fn msg_set_isize(obj: *mut std::ffi::c_void, name: &str, val: isize) {
    let f: unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8, isize) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, sel(name), val) }
}

fn msg_set_bool(obj: *mut std::ffi::c_void, name: &str, val: bool) {
    let f: unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8, bool) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, sel(name), val) }
}

pub fn raise(w: &slint::Window) {
    w.set_minimized(false);
    w.with_winit_window(|w| w.focus_window());
}

pub fn scale(w: &slint::Window) -> f32 {
    w.with_winit_window(|w| w.scale_factor() as f32).unwrap_or(1.0)
}

/// Монитор с окном целиком. Если winit монитор ещё не знает — отсчёт от размера окна.
pub fn work_area(w: &slint::Window) -> (i32, i32, i32, i32) {
    w.with_winit_window(|w| {
        let m = w.current_monitor()?;
        let (p, s) = (m.position(), m.size());
        Some((p.x, p.y, p.x + s.width as i32, p.y + s.height as i32))
    })
    .flatten()
    .unwrap_or_else(|| {
        let s = w.size();
        (0, 0, s.width as i32, s.height as i32)
    })
}

/// Строка AppleScript в кавычках.
fn apple_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn osascript(script: &str) -> Option<String> {
    let out = Command::new("osascript").arg("-e").arg(script).stderr(Stdio::null()).output().ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !path.is_empty()).then_some(path)
}

pub fn pick_image(_: &slint::Window, title: &str) -> Option<String> {
    let script = format!(
        "POSIX path of (choose file with prompt {} of type {{\"public.jpeg\", \"public.png\", \"jpeg\", \"jpg\", \"png\"}})",
        apple_quote(title)
    );
    osascript(&script)
}

pub fn pick_folder(_: &slint::Window, title: &str, start: &str) -> Option<String> {
    let start = Path::new(start);
    let extra = if start.is_dir() {
        format!(" default location POSIX file {}", apple_quote(&start.to_string_lossy()))
    } else {
        String::new()
    };
    let path = osascript(&format!("POSIX path of (choose folder with prompt {}{extra})", apple_quote(title)))?;
    let path = path.trim_end_matches('/').to_string();
    (!path.is_empty()).then_some(path)
}

pub fn open_url(url: &str) {
    if let Ok(mut child) = Command::new("open").arg(url).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

/// У glibc есть malloc_trim, у системного malloc macOS — нет.
pub fn trim_memory() {}

pub fn release_memory() {
    trim_memory();
}

/// «Как в системе»: ключ AppleInterfaceStyle есть только в тёмной теме.
/// Нет ключа — светлая, как Windows без записи в реестре.
pub fn system_light_theme() -> bool {
    let out = Command::new("defaults").args(["read", "-g", "AppleInterfaceStyle"]).stderr(Stdio::null()).output();
    !matches!(out, Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("dark"))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

pub fn config_base() -> PathBuf {
    home().join("Library").join("Application Support")
}

pub fn cache_base() -> PathBuf {
    home().join("Library").join("Caches")
}

pub fn music_dir() -> PathBuf {
    home().join("Music")
}

/// Имя компьютера, как в «Об этом Mac». Им подписывается устройство в аккаунте Яндекса.
pub fn hostname() -> Option<String> {
    let from = |cmd: &str, args: &[&str]| {
        let out = Command::new(cmd).args(args).stderr(Stdio::null()).output().ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !name.is_empty()).then_some(name)
    };
    from("scutil", &["--get", "ComputerName"]).or_else(|| from("hostname", &[]))
}

pub fn copy_text(text: &str) {
    let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
}

/// `~/Library/LaunchAgents/yamusic-rs.plist`. Трея нет, поэтому при входе открывается окно.
fn plist_path() -> PathBuf {
    home().join("Library").join("LaunchAgents").join(format!("{APP_ID}.plist"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn user_id() -> Option<String> {
    let out = Command::new("id").arg("-u").stderr(Stdio::null()).output().ok()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && id.chars().all(|c| c.is_ascii_digit())).then_some(id)
}

pub fn autostart_enabled() -> bool {
    plist_path().is_file()
}

/// Запись в LaunchAgents. `launchctl bootstrap` не зовём: с `RunAtLoad` он сразу открыл бы
/// вторую копию. Файл подхватится при следующем входе. При выключении выгружаем агент, если
/// он уже был загружен в этом сеансе.
pub fn set_autostart(on: bool) -> bool {
    let file = plist_path();
    if !on {
        let removed = std::fs::remove_file(&file).is_ok() || !file.exists();
        if let Some(uid) = user_id() {
            let _ = Command::new("launchctl")
                .args(["bootout", &format!("gui/{uid}/{APP_ID}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        return removed;
    }
    let Some(exe) = std::env::current_exe().ok() else { return false };
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{APP_ID}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#,
        xml_escape(&exe.display().to_string())
    );
    file.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&file, body).is_ok()
}

pub fn refresh_autostart() {
    if autostart_enabled() {
        set_autostart(true);
    }
}

/// Личная временная папка пользователя (`/var/folders/.../T`), не общий /tmp.
fn socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("{APP_ID}-{}.sock", std::env::var("USER").unwrap_or_default()))
}

static LISTENER: Mutex<Option<UnixListener>> = Mutex::new(None);

/// Занять сокет единственного экземпляра. Сокет остаётся на диске и после падения,
/// поэтому занятый, но молчащий файл считается брошенным.
pub fn claim_instance() -> bool {
    let path = socket_path();
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(&path).is_ok() {
                return false;
            }
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(_) => return true,
            }
        }
        Err(_) => return true,
    };
    *LISTENER.lock().unwrap() = Some(listener);
    true
}

pub fn wake_existing() {
    if let Ok(mut s) = UnixStream::connect(socket_path()) {
        let _ = s.write_all(b"show");
    }
}

/// Звать `f` из фонового потока, когда второй запуск просит показать окно.
/// Пустое соединение — это проверка из `claim_instance`, на неё не реагируем.
pub fn on_wake(f: impl Fn() + Send + 'static) {
    let Some(listener) = LISTENER.lock().unwrap().take() else { return };
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 4];
            if s.read(&mut buf).is_ok_and(|n| n > 0) {
                f();
            }
        }
    });
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
