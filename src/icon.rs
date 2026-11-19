//! 앱 메뉴 아이콘. 카카오톡 로고는 저장소에 넣지 않고, 사용자 PC에 설치된 exe 에서 추출한다.

use crate::util;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const NAME: &str = "kakaotalk";
/// 카카오톡 아이콘을 못 구했을 때 쓸 테마 기본 아이콘
pub const FALLBACK: &str = "internet-chat";

fn search_dirs() -> Vec<PathBuf> {
    let home = util::home();
    vec![
        util::data_home().join("icons"),
        home.join(".icons"),
        util::data_home().join("pixmaps"),
        PathBuf::from("/usr/share/icons/hicolor"),
        PathBuf::from("/usr/share/pixmaps"),
    ]
}

fn find(dir: &Path, depth: u32) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0
                && let Some(f) = find(&p, depth - 1)
            {
                return Some(f);
            }
        } else if p.file_stem().is_some_and(|s| s == NAME)
            && p.extension()
                .is_some_and(|x| x == "png" || x == "svg" || x == "xpm")
        {
            return Some(p);
        }
    }
    None
}

pub fn installed() -> Option<PathBuf> {
    search_dirs().iter().find_map(|d| find(d, 4))
}

/// .desktop 의 Icon= 값
pub fn icon_name() -> &'static str {
    if installed().is_some() {
        NAME
    } else {
        FALLBACK
    }
}

/// 이미 있으면 그대로, 없으면 설치된 카카오톡 exe 에서 추출해
/// ~/.local/share/icons/hicolor/NxN/apps/kakaotalk.png 로 설치
pub fn ensure_icon(exe: &Path) -> Result<PathBuf, String> {
    if let Some(p) = installed() {
        return Ok(p);
    }
    extract_icon(exe)
}

/// 이 런처가 만든 아이콘을 지우고 다시 추출 (수리 탭)
pub fn regenerate(exe: &Path) -> Result<PathBuf, String> {
    let hicolor = util::data_home().join("icons/hicolor");
    if let Ok(rd) = fs::read_dir(&hicolor) {
        for e in rd.flatten() {
            let _ = fs::remove_file(e.path().join("apps").join(format!("{NAME}.png")));
        }
    }
    extract_icon(exe)
}

/// 수리 탭 표시용: (정상 여부, 설명)
pub fn status() -> (bool, String) {
    match installed() {
        Some(_) => (true, "✓ 카카오톡 아이콘이 적용돼 있습니다".into()),
        None if !tools_ok() => (false, TOOLS_MSG.into()),
        None => (false, "아이콘 없음 — '아이콘 고치기'를 누르세요".into()),
    }
}

const TOOLS_MSG: &str = "아이콘 추출에 icoutils 가 필요합니다 (sudo apt install icoutils)";

fn tools_ok() -> bool {
    util::which("wrestool").is_some() && util::which("icotool").is_some()
}

/// 아이콘을 찾아볼 exe 목록: KakaoTalkUI.exe → 같은 폴더/상위 폴더의 카카오톡 exe → 나머지 exe
fn candidates(exe: &Path) -> Vec<PathBuf> {
    let mut out = vec![exe.to_path_buf()];
    let dirs: Vec<PathBuf> = [exe.parent(), exe.parent().and_then(Path::parent)]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect();
    let mut rest: Vec<PathBuf> = Vec::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.ends_with(".exe") || out.contains(&p) || rest.contains(&p) {
                continue;
            }
            // 크래시 리포터·제거 프로그램은 마지막으로
            if name.contains("kakao") && !name.contains("crash") && !name.contains("unins") {
                out.push(p);
            } else {
                rest.push(p);
            }
        }
    }
    out.extend(rest);
    out
}

fn extract_icon(exe: &Path) -> Result<PathBuf, String> {
    if !exe.is_file() {
        return Err("카카오톡이 아직 설치되지 않았습니다".into());
    }
    if !tools_ok() {
        return Err(TOOLS_MSG.into());
    }
    let mut errs = Vec::new();
    for c in candidates(exe) {
        match extract_from(&c) {
            Ok(p) => return Ok(p),
            Err(e) => errs.push(format!(
                "{}: {e}",
                c.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    Err(format!(
        "아이콘을 추출하지 못했습니다 ({})",
        errs.join(" / ")
    ))
}

/// 아이콘 테마(hicolor)가 인식하는 크기 폴더로 맞춤
fn theme_size(px: u32) -> u32 {
    const SIZES: [u32; 10] = [16, 22, 24, 32, 48, 64, 96, 128, 256, 512];
    SIZES.iter().rev().copied().find(|s| *s <= px).unwrap_or(16)
}

fn extract_from(exe: &Path) -> Result<PathBuf, String> {
    let work = util::state_dir().join("icon-extract");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    // RT_GROUP_ICON(14) 리소스 → .ico
    let out = Command::new("wrestool")
        .args(["-x", "-t", "14", "-o"])
        .arg(&work)
        .arg(exe)
        .stdout(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let icos: Vec<PathBuf> = fs::read_dir(&work)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ico")))
        .collect();
    if icos.is_empty() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if why.is_empty() {
            "아이콘 리소스 없음".into()
        } else {
            why
        });
    }
    let png_dir = work.join("png");
    fs::create_dir_all(&png_dir).map_err(|e| e.to_string())?;
    for ico in &icos {
        let _ = Command::new("icotool")
            .arg("-x")
            .arg("-o")
            .arg(&png_dir)
            .arg(ico)
            .stderr(Stdio::null())
            .status();
    }
    // 가장 큰 PNG
    let best = fs::read_dir(&png_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| png_size(&p).map(|s| (s, p)))
        .max_by_key(|(s, _)| *s)
        .ok_or("ico 를 PNG 로 바꾸지 못했습니다")?;
    let size = theme_size(best.0);
    let hicolor = util::data_home().join("icons/hicolor");
    let dst_dir = hicolor.join(format!("{size}x{size}/apps"));
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let dst = dst_dir.join(format!("{NAME}.png"));
    fs::copy(&best.1, &dst).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&work);
    // 오래된 아이콘 캐시가 새 아이콘을 가리지 않도록 갱신 (캐시가 이미 있을 때만 의미 있음)
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t", "-q"])
        .arg(&hicolor)
        .stderr(Stdio::null())
        .status();
    Ok(dst)
}

/// PNG 헤더(IHDR)에서 가로 크기
fn png_size(p: &Path) -> Option<u32> {
    let data = fs::read(p).ok()?;
    if data.len() < 24 || &data[1..4] != b"PNG" {
        return None;
    }
    Some(u32::from_be_bytes(data[16..20].try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_theme_sizes() {
        assert_eq!(theme_size(256), 256);
        assert_eq!(theme_size(40), 32);
        assert_eq!(theme_size(1024), 512);
        assert_eq!(theme_size(8), 16);
    }

    #[test]
    fn candidate_order() {
        let r = std::env::temp_dir().join(format!("ktl-icon-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let bin = r.join("KakaoTalk/bin");
        fs::create_dir_all(&bin).unwrap();
        for f in [
            "bin/KakaoTalkUI.exe",
            "bin/CrashReporter.exe",
            "KakaoTalk.exe",
            "uninstall.exe",
        ] {
            fs::write(r.join("KakaoTalk").join(f), "").unwrap();
        }
        let c = candidates(&bin.join("KakaoTalkUI.exe"));
        let names: Vec<String> = c
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names[0], "KakaoTalkUI.exe");
        assert_eq!(names[1], "KakaoTalk.exe");
        assert!(names.iter().position(|n| n == "CrashReporter.exe").unwrap() > 1);
        let _ = fs::remove_dir_all(&r);
    }
}
