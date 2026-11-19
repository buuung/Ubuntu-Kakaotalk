//! umu-run 확보: 자체 경로 → Lutris 복사본 → PATH, 없으면 GitHub 에서 받기

use crate::config::Config;
use crate::util;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

const RELEASE_API: &str =
    "https://api.github.com/repos/Open-Wine-Components/umu-launcher/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/Open-Wine-Components/umu-launcher/releases";

/// 실패했을 때 보여줄 수동 설치 방법
pub fn manual_hint() -> String {
    format!(
        "{RELEASES_PAGE} 에서 'zipapp' 이 들어간 압축 파일을 받아 풀고, 안의 umu-run 을 {} 에 넣으세요",
        own_path().display()
    )
}

pub fn own_dir() -> PathBuf {
    util::data_dir().join("umu")
}
pub fn own_path() -> PathBuf {
    own_dir().join("umu-run")
}
pub fn lutris_dir() -> PathBuf {
    util::data_home().join("lutris/runtime/umu")
}

pub fn resolve(cfg: &Config) -> Option<PathBuf> {
    let custom = cfg.umu_run.trim();
    if !custom.is_empty() {
        let p = util::expand(custom);
        return p.is_file().then_some(p);
    }
    [own_path(), lutris_dir().join("umu-run")]
        .into_iter()
        .find(|p| p.is_file())
        .or_else(|| util::which("umu-run"))
}

/// 자체 경로에 umu-run 이 없고 Lutris 에 있으면 복사 (Lutris 를 지워도 동작하도록)
pub fn ensure_own_copy() -> Result<Option<PathBuf>, String> {
    if own_path().is_file() {
        return Ok(Some(own_path()));
    }
    if lutris_dir().join("umu-run").is_file() {
        return copy_from_lutris().map(Some);
    }
    Ok(None)
}

pub fn copy_from_lutris() -> Result<PathBuf, String> {
    let src = lutris_dir();
    if !src.join("umu-run").is_file() {
        return Err(format!("{} 에 umu-run 이 없습니다", src.display()));
    }
    let dst = own_dir();
    fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
    let st = Command::new("cp")
        .arg("-a")
        .arg(format!("{}/.", src.display()))
        .arg(&dst)
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("umu 폴더 복사 실패".into());
    }
    Ok(own_path())
}

/// GitHub 최신 릴리스의 zipapp 을 받아 자체 경로에 설치. 반환: 태그 이름
pub fn download_latest() -> Result<String, String> {
    download_latest_inner().map_err(|e| format!("{e}\n수동 설치: {}", manual_hint()))
}

fn download_latest_inner() -> Result<String, String> {
    // 테스트용: 다른 릴리스 JSON 주소로 바꿀 수 있음
    let api = std::env::var("KTL_UMU_RELEASE_API").unwrap_or_else(|_| RELEASE_API.into());
    let rel = util::fetch_json(&api)?;
    let tag = rel["tag_name"].as_str().unwrap_or("?").to_string();
    let urls: Vec<String> = rel["assets"]
        .as_array()
        .ok_or_else(|| {
            format!(
                "GitHub 응답 오류: {}",
                rel["message"].as_str().unwrap_or("?")
            )
        })?
        .iter()
        .filter_map(|a| a["browser_download_url"].as_str().map(String::from))
        .collect();
    let url = pick_zipapp(&urls).ok_or("릴리스에서 zipapp 압축 파일을 찾지 못했습니다")?;

    let work = util::state_dir().join("umu-download");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let tar = work.join("umu.tar");
    util::download(&url, &tar)?;
    let st = Command::new("tar")
        .arg("-xf")
        .arg(&tar)
        .arg("-C")
        .arg(&work)
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("압축 해제 실패".into());
    }
    let found = find_file(&work, "umu-run").ok_or("압축 안에 umu-run 이 없습니다")?;
    let dst = own_dir();
    fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
    // 같은 폴더의 파일을 통째로 복사 (zipapp 은 보통 umu-run 하나)
    let st = Command::new("cp")
        .arg("-a")
        .arg(format!("{}/.", found.parent().unwrap().display()))
        .arg(&dst)
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("umu-run 복사 실패".into());
    }
    let _ = fs::set_permissions(own_path(), fs::Permissions::from_mode(0o755));
    let _ = fs::remove_dir_all(&work);
    Ok(tag)
}

/// 릴리스 파일 중 zipapp 압축본 (umu-launcher-X.Y.Z-zipapp.tar 등, 압축 형식 무관)
fn pick_zipapp(urls: &[String]) -> Option<String> {
    let is_tar = |u: &str| {
        [".tar", ".tar.gz", ".tgz", ".tar.xz", ".tar.zst", ".tar.bz2"]
            .iter()
            .any(|x| u.ends_with(x))
    };
    urls.iter()
        .find(|u| {
            let n = u.rsplit('/').next().unwrap_or(u).to_ascii_lowercase();
            n.contains("zipapp") && is_tar(&n)
        })
        .cloned()
}

fn find_file(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if e.file_name() == name {
            return Some(p);
        }
    }
    None
}

/// umu 가 관리하는 Steam Runtime 이 이미 받아져 있는지
pub fn steam_runtime_present() -> bool {
    fs::read_dir(util::data_home().join("umu"))
        .map(|rd| {
            rd.flatten().any(|e| {
                e.file_name().to_string_lossy().starts_with("steamrt") && e.path().is_dir()
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_zipapp_variants() {
        let u = |n: &str| format!("https://github.com/x/releases/download/1.2.9/{n}");
        let list = vec![
            u("umu-launcher-1.2.9-artifacts.zip"),
            u("python3-umu-launcher_1.2.9-1_amd64.deb"),
            u("umu-launcher-1.2.9-zipapp.tar"),
        ];
        assert!(pick_zipapp(&list).unwrap().ends_with("zipapp.tar"));
        assert!(pick_zipapp(&[u("umu-launcher-2.0-zipapp.tar.gz")]).is_some());
        assert!(pick_zipapp(&[u("umu-launcher-2.0.deb")]).is_none());
    }
}
