//! Winemoji 폰트 설치/업데이트 (https://github.com/kmbzn/project-winemoji)

use crate::util;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const CONTENTS_API: &str = "https://api.github.com/repos/kmbzn/project-winemoji/contents/fonts";

#[derive(Debug, Clone)]
pub struct RemoteFont {
    pub name: String,
    pub url: String,
    pub size: u64,
}

pub fn font_dir() -> PathBuf {
    util::data_home().join("fonts/winemoji")
}

/// (크기, 수정 시각)
pub fn installed(name: &str) -> Option<(u64, String)> {
    let p = font_dir().join(name);
    let m = fs::metadata(&p).ok()?;
    Some((m.len(), util::file_mtime_string(&p)))
}

fn is_font(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".ttf") || n.ends_with(".otf") || n.ends_with(".ttc")
}

pub fn list_remote() -> Result<Vec<RemoteFont>, String> {
    let mut out = Vec::new();
    collect(CONTENTS_API, &mut out, 0)?;
    out.sort_by(|a, b| a.name.cmp(&b.name));
    if out.is_empty() {
        return Err("저장소 fonts/ 에서 폰트 파일을 찾지 못했습니다".into());
    }
    Ok(out)
}

fn collect(url: &str, out: &mut Vec<RemoteFont>, depth: u32) -> Result<(), String> {
    let v = util::fetch_json(url)?;
    let arr = v
        .as_array()
        .ok_or_else(|| format!("GitHub 응답 오류: {}", v["message"].as_str().unwrap_or("?")))?;
    for item in arr {
        let name = item["name"].as_str().unwrap_or_default();
        match item["type"].as_str() {
            Some("file") if is_font(name) => {
                if let Some(dl) = item["download_url"].as_str() {
                    out.push(RemoteFont {
                        name: name.to_string(),
                        url: dl.to_string(),
                        size: item["size"].as_u64().unwrap_or(0),
                    });
                }
            }
            Some("dir") if depth < 1 => {
                if let Some(u) = item["url"].as_str() {
                    collect(u, out, depth + 1)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// 최신 파일을 받아 교체 후 fc-cache. 반환: 사람이 읽을 결과 메시지
pub fn install(name: &str) -> Result<String, String> {
    let list = list_remote()?;
    let f = list
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("원격 저장소에 {name} 이(가) 없습니다"))?;

    let dir = font_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".{name}.download"));
    util::download(&f.url, &tmp)?;

    let data = fs::read(&tmp).map_err(|e| e.to_string())?;
    let magic_ok = data.len() > 4
        && (data[..4] == [0, 1, 0, 0]
            || &data[..4] == b"OTTO"
            || &data[..4] == b"true"
            || &data[..4] == b"ttcf");
    if !magic_ok || (f.size > 0 && data.len() as u64 != f.size) {
        let _ = fs::remove_file(&tmp);
        return Err("받은 파일이 올바른 폰트가 아닙니다".into());
    }

    let dst = dir.join(name);
    let same = fs::read(&dst).is_ok_and(|old| old == data);
    if same {
        let _ = fs::remove_file(&tmp);
    } else {
        fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    }
    let _ = Command::new("fc-cache")
        .arg("-fv")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(if same {
        format!(
            "{name} 은(는) 이미 최신입니다 ({} bytes). 폰트 캐시를 갱신했습니다.",
            data.len()
        )
    } else {
        format!(
            "{name} 설치 완료 ({} bytes). 카카오톡 설정 → 화면 → 글씨체에서 선택하세요.",
            data.len()
        )
    })
}
