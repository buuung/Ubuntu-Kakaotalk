//! 앱 메뉴 바로가기 (~/.local/share/applications/kakaotalk.desktop)

use crate::util;
use std::fs;
use std::path::{Path, PathBuf};

pub fn desktop_path() -> PathBuf {
    util::data_home().join("applications/kakaotalk.desktop")
}

pub fn current_exec() -> Option<String> {
    let c = fs::read_to_string(desktop_path()).ok()?;
    c.lines()
        .take_while(|l| !l.starts_with("[Desktop Action"))
        .find_map(|l| l.strip_prefix("Exec=").map(str::to_string))
}

pub fn points_to(exe: &Path) -> bool {
    current_exec().is_some_and(|e| e.contains(&*exe.to_string_lossy()))
}

fn quote(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.into_owned()
    }
}

/// 기존 파일은 kakaotalk.desktop.bak 로 백업 후 교체
pub fn install(exe: &Path) -> Result<PathBuf, String> {
    let p = desktop_path();
    fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    if let Ok(old) = fs::read_to_string(&p) {
        let bak = p.with_extension("desktop.bak");
        if !bak.exists() {
            fs::write(&bak, old).map_err(|e| e.to_string())?;
        }
    }
    let x = quote(exe);
    // 카카오톡이 설치돼 있으면 exe 에서 아이콘 추출 (실패해도 기본 아이콘으로 진행)
    let _ = crate::icon::ensure_icon(&crate::config::Config::load().exe_path());
    let icon = crate::icon::icon_name();
    let content = format!(
        "[Desktop Entry]
Name=KakaoTalk
Comment=KakaoTalk
Exec={x} --launch
Icon={icon}
Terminal=false
Type=Application
Categories=Network;InstantMessaging;
StartupNotify=false
StartupWMClass=kakaotalkui.exe
Actions=settings;kill;

[Desktop Action settings]
Name=런처 설정
Exec={x} --settings

[Desktop Action kill]
Name=강제 종료
Exec={x} --kill
"
    );
    util::write_atomic(&p, content.as_bytes())?;
    let _ = std::process::Command::new("update-desktop-database")
        .arg(p.parent().unwrap())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(p)
}

/// 정리 대상 후보 (삭제는 하지 않고 알려주기만)
pub fn leftovers() -> Vec<PathBuf> {
    let home = util::home();
    [
        home.join(".var/app/com.usebottles.bottles/data/bottles/bottles/KakaoTalk"),
        home.join(".local/bin/kakaotalk-launcher"),
    ]
    .into_iter()
    .filter(|p| p.exists())
    .collect()
}
