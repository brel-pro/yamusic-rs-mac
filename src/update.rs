//! Обновления через релизы GitHub, как у оригинала: проверка раз в полчаса, тост
//! «Доступна версия …» с кнопкой «Обновить». Приложение — один exe, поэтому кнопка
//! скачивает `yamusic-rs.exe` нового релиза, сверяет SHA-256 с тем, что показывает GitHub,
//! и подменяет собой запущенный файл: Windows не даёт перезаписать работающий exe, но даёт
//! его переименовать. Старый файл (`*.old.exe`) удаляется при следующем запуске.
//! Если папка недоступна для записи, кнопка открывает страницу релиза. На Linux — всегда её.
//! На macOS скачивается zip с пакетом .app: папка работающего приложения переименовывается,
//! на её место встаёт новая, процесс перезапускается.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Как часто спрашивать GitHub. У оригинала UPDATE_POLL_INTERVAL_MS — те же 30 минут.
pub const POLL: Duration = Duration::from_secs(30 * 60);

/// Имя exe в релизе Windows.
const ASSET: &str = concat!(env!("CARGO_PKG_NAME"), ".exe");
/// Архив пакета macOS.
const MAC_ASSET: &str = "yamusic-rs-macos-aarch64.zip";

/// Аргумент, с которым обновлённый exe ждёт, пока закроется старый.
pub const AFTER_UPDATE: &str = "--after-update";

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// Страница релиза на GitHub.
    pub page: String,
    /// Exe: адрес, размер и SHA-256. Нет — обновить можно только руками.
    pub exe: Option<(String, u64, String)>,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        // GitHub API без User-Agent отвечает 403.
        .user_agent(concat!("yamusic-rs/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}

/// «v1.2.3» / «1.2.3» → (1, 2, 3). Пре-релизы (`/releases/latest` их и так не отдаёт) не нужны.
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().trim_start_matches('v').split(['.', '-']).map(|p| p.parse::<u32>());
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next().unwrap_or(Ok(0)).ok()?))
}

/// Куда смотреть обновления. На macOS это форк: исходный репозиторий macOS не выпускает.
/// Windows и Linux остаются на адресе из Cargo.toml.
pub fn project_url() -> &'static str {
    if cfg!(target_os = "macos") {
        "https://github.com/brel-pro/yamusic-rs-mac"
    } else {
        env!("CARGO_PKG_REPOSITORY")
    }
}

/// Последний релиз, если он новее запущенной версии.
pub fn check() -> Result<Option<Release>, String> {
    check_repo(project_url(), env!("CARGO_PKG_VERSION"))
}

fn check_repo(repo: &str, current: &str) -> Result<Option<Release>, String> {
    let api = repo.replace("https://github.com/", "https://api.github.com/repos/");
    let resp = client()?.get(format!("{api}/releases/latest")).send().map_err(|e| crate::config::chain(&e))?;
    // 404 — релизов ещё нет.
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub: HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).ok_or("в релизе нет версии")?;
    let (Some(latest), Some(current)) = (parse_version(tag), parse_version(current)) else {
        return Err(format!("непонятная версия {tag}"));
    };
    if latest <= current {
        return Ok(None);
    }
    // На Linux файл не нужен: новый пакет ставится со страницы релиза.
    let asset_name = if cfg!(target_os = "macos") {
        Some(MAC_ASSET)
    } else if cfg!(windows) {
        Some(ASSET)
    } else {
        None
    };
    let exe = asset_name.and_then(|name| {
        let assets = v.get("assets")?.as_array()?;
        let a = assets.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some(name))?;
        let url = a.get("browser_download_url")?.as_str()?.to_string();
        let size = a.get("size")?.as_u64()?;
        let sha = a.get("digest")?.as_str()?.strip_prefix("sha256:")?.to_ascii_lowercase();
        Some((url, size, sha))
    });
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        // Страницу откроет браузер системы: адрес из ответа берём, только если это https.
        page: v
            .get("html_url")
            .and_then(|u| u.as_str())
            .and_then(crate::client::browser_url)
            .unwrap_or_else(|| repo.to_string()),
        exe,
    }))
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("old.exe")
}

/// Старый пакет уезжает в `Имя.app.old`, новый встаёт на его место. Если вторая
/// перестановка не удалась, старый пакет возвращается обратно.
fn replace_bundle(app: &Path, new_app: &Path) -> Result<(), String> {
    let name = app.file_name().ok_or("нет имени пакета")?.to_string_lossy();
    let old = app.with_file_name(format!("{name}.old"));
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(app, &old).map_err(|e| format!("не удалось заменить приложение: {e}"))?;
    if let Err(e) = std::fs::rename(new_app, app) {
        let _ = std::fs::rename(&old, app);
        return Err(format!("не удалось заменить приложение: {e}"));
    }
    Ok(())
}

/// Скачать новый файл, проверить и поставить на место запущенного. Долго: звать из фонового
/// потока. После успеха нужно запустить [`relaunch`] и закрыться.
pub fn install(r: &Release) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        return install_macos(r);
    }
    #[cfg(not(target_os = "macos"))]
    install_exe(r)
}

fn download(url: &str, size: u64, sha: &str) -> Result<Vec<u8>, String> {
    let bytes = client()?
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| format!("загрузка: {}", crate::config::chain(&e)))?;
    if bytes.len() as u64 != size {
        return Err("файл скачался не целиком".into());
    }
    let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got != sha {
        return Err("контрольная сумма не совпала".into());
    }
    Ok(bytes.to_vec())
}

/// `…/Имя.app/Contents/MacOS/программа` → `…/Имя.app`. Голый бинарник — не пакет.
fn bundle_root(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name() == Some(std::ffi::OsStr::new("MacOS"))
        && contents.file_name() == Some(std::ffi::OsStr::new("Contents"))
        && app.extension() == Some(std::ffi::OsStr::new("app")))
    .then(|| app.to_path_buf())
}

#[cfg(target_os = "macos")]
fn old_bundle(exe: &Path) -> Option<PathBuf> {
    let app = bundle_root(exe)?;
    let name = app.file_name()?.to_string_lossy();
    Some(app.with_file_name(format!("{name}.old")))
}

#[cfg(target_os = "macos")]
fn install_macos(r: &Release) -> Result<PathBuf, String> {
    let (url, size, sha) = r.exe.as_ref().ok_or("у релиза нет архива")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let app = bundle_root(&exe).ok_or("это не пакет .app, обновить можно только со страницы релиза")?;
    let parent = app.parent().ok_or("нет папки пакета")?.to_path_buf();
    let bytes = download(url, *size, sha)?;
    let stage = parent.join(".yamusic-rs-update");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("нет доступа к папке {}: {e}", parent.display()))?;
    let zip = stage.join("update.zip");
    if let Err(e) = std::fs::write(&zip, &bytes) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(format!("нет доступа к папке {}: {e}", parent.display()));
    }
    let unpacked = std::process::Command::new("ditto").args(["-x", "-k"]).arg(&zip).arg(&stage).status();
    if !unpacked.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("архив не распаковался".into());
    }
    let Some(new_app) = find_app(&stage) else {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("в архиве нет приложения".into());
    };
    if !new_app.join("Contents/MacOS/yamusic-rs").is_file() {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("в новом пакете нет программы".into());
    }
    if let Err(e) = replace_bundle(&app, &new_app) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&stage);
    Ok(app.join("Contents/MacOS/yamusic-rs"))
}

#[cfg(target_os = "macos")]
fn find_app(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.extension() == Some(std::ffi::OsStr::new("app")))
}

#[cfg(not(target_os = "macos"))]
fn install_exe(r: &Release) -> Result<PathBuf, String> {
    let (url, size, sha) = r.exe.as_ref().ok_or("у релиза нет exe")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bytes = download(url, *size, sha)?;
    let new = exe.with_extension("new.exe");
    std::fs::write(&new, bytes).map_err(|e| format!("нет доступа к папке {}: {e}", exe.parent().unwrap_or(&exe).display()))?;
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(&exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("не удалось заменить exe: {e}"));
    }
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("не удалось заменить exe: {e}"));
    }
    Ok(exe)
}

/// Запустить обновлённый exe. Он дождётся, пока этот процесс закроется.
pub fn relaunch(exe: &Path) -> Result<(), String> {
    std::process::Command::new(exe)
        .arg(AFTER_UPDATE)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("новая версия не запустилась: {e}"))
}

/// Убрать exe прошлой версии, оставшийся после обновления. Старый процесс может ещё
/// завершаться и держать файл, поэтому пробуем несколько раз в фоне.
fn leftover(exe: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(old) = old_bundle(exe) {
        return old;
    }
    old_path(exe)
}

pub fn cleanup() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = leftover(&exe);
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..15 {
            let gone = if old.is_dir() { std::fs::remove_dir_all(&old).is_ok() } else { std::fs::remove_file(&old).is_ok() };
            if gone || !old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert!(parse_version("v0.10.0") > parse_version("0.9.9"));
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn mac_does_not_follow_upstream() {
        if cfg!(target_os = "macos") {
            assert_eq!(project_url(), "https://github.com/brel-pro/yamusic-rs-mac");
            assert!(!project_url().contains("SpolzerMod"));
        } else {
            assert_eq!(project_url(), env!("CARGO_PKG_REPOSITORY"));
        }
    }

    /// Живой запрос. Не в обычном `cargo test`: зависит от GitHub.
    #[test]
    #[ignore]
    fn live_fork_is_newer_than_1_0() {
        let found = check_repo(project_url(), "1.0.0").expect("запрос");
        let found = found.expect("релиз новее 1.0.0");
        assert!(found.page.starts_with("https://github.com/brel-pro/yamusic-rs-mac/"));
        assert!(found.exe.as_ref().is_some_and(|(url, _, _)| url.ends_with(MAC_ASSET)));
        assert!(parse_version(&found.version) > parse_version("1.0.0"));
    }

    /// Сборка 1.1.0 с этим кодом должна увидеть релиз форка и архив пакета.
    #[test]
    #[ignore]
    fn live_110_is_offered_our_release() {
        let found = check_repo(project_url(), "1.1.0").expect("запрос").expect("релиз новее 1.1.0");
        assert!(found.page.starts_with("https://github.com/brel-pro/yamusic-rs-mac/"));
        assert!(found.exe.as_ref().is_some_and(|(url, _, _)| url.ends_with(MAC_ASSET)));
        assert!(parse_version(&found.version) >= parse_version("1.1.1"));
    }

    #[test]
    fn paths() {
        assert_eq!(old_path(Path::new(r"C:\a\yamusic-rs.exe")), Path::new(r"C:\a\yamusic-rs.old.exe"));
        assert_eq!(ASSET, "yamusic-rs.exe");
        let exe = Path::new("/Applications/Яндекс Музыка RS.app/Contents/MacOS/yamusic-rs");
        assert_eq!(bundle_root(exe).as_deref(), Some(Path::new("/Applications/Яндекс Музыка RS.app")));
        assert!(bundle_root(Path::new("/tmp/yamusic-rs")).is_none());
    }

    #[test]
    fn bundle_swap_puts_new_app_in_place() {
        let root = std::env::temp_dir().join(format!("yamusic-rs-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = root.join("Демо.app");
        let new_app = root.join("stage").join("Другое.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/yamusic-rs"), b"old").unwrap();
        std::fs::create_dir_all(new_app.join("Contents/MacOS")).unwrap();
        std::fs::write(new_app.join("Contents/MacOS/yamusic-rs"), b"new").unwrap();
        replace_bundle(&app, &new_app).unwrap();
        assert_eq!(std::fs::read(app.join("Contents/MacOS/yamusic-rs")).unwrap(), b"new");
        assert!(!new_app.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
