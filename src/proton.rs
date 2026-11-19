//! Proton 버전 탐색

use crate::util;
use std::fs;
use std::path::{Path, PathBuf};

pub fn compat_dirs() -> Vec<PathBuf> {
    let home = util::home();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for d in [
        util::data_home().join("Steam/compatibilitytools.d"),
        home.join(".steam/root/compatibilitytools.d"),
        home.join(".steam/steam/compatibilitytools.d"),
    ] {
        let canon = fs::canonicalize(&d).unwrap_or(d.clone());
        if d.is_dir()
            && !dirs
                .iter()
                .any(|x| fs::canonicalize(x).unwrap_or(x.clone()) == canon)
        {
            dirs.push(d);
        }
    }
    dirs
}

/// (폴더 이름, 경로) — 이름 내림차순(최신 우선)
pub fn scan() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for d in compat_dirs() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.join("proton").is_file() {
                let name = e.file_name().to_string_lossy().to_string();
                if !out.iter().any(|(n, _)| *n == name) {
                    out.push((name, p));
                }
            }
        }
    }
    out.sort_by(|a, b| {
        // CachyOS 를 맨 위로, 그 다음 이름 내림차순
        is_cachyos(&b.0).cmp(&is_cachyos(&a.0)).then(b.0.cmp(&a.0))
    });
    out
}

pub fn resolve(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        let p = util::expand(name);
        return p.join("proton").is_file().then_some(p);
    }
    scan().into_iter().find(|(n, _)| n == name).map(|(_, p)| p)
}

pub fn is_cachyos(name: &str) -> bool {
    name.to_ascii_lowercase().contains("cachyos")
}

pub fn is_ge(name: &str) -> bool {
    name.to_ascii_lowercase().contains("ge-proton")
}

/// 기본 선택: 설정값이 있으면 그것, 없으면 최신 Proton-CachyOS
pub fn pick_default(list: &[(String, PathBuf)], wanted: &str) -> Option<String> {
    if list.iter().any(|(n, _)| n == wanted) {
        return Some(wanted.to_string());
    }
    list.iter()
        .find(|(n, _)| is_cachyos(n))
        .or(list.first())
        .map(|(n, _)| n.clone())
}

pub fn wineserver(proton: &Path) -> Option<PathBuf> {
    ["files/bin/wineserver", "dist/bin/wineserver"]
        .iter()
        .map(|r| proton.join(r))
        .find(|p| p.is_file())
}

/// Proton 에 대한 한 줄 안내
pub fn hint(name: &str) -> &'static str {
    if is_cachyos(name) {
        "H.264 디코더 포함 — 동영상 전송 OK (권장)"
    } else if is_ge(name) {
        "⚠ GE-Proton: H.264 디코더가 없어 동영상 전송 시 크래시합니다"
    } else {
        "검증되지 않은 Proton 입니다"
    }
}

// ─────────────────── Proton-CachyOS 다운로드 ───────────────────

const CACHYOS_API: &str = "https://api.github.com/repos/CachyOS/proton-cachyos/releases/latest";

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sum_url: Option<String>,
}

impl Asset {
    /// 압축을 푼 폴더 이름
    pub fn dir_name(&self) -> String {
        self.name
            .trim_end_matches(".tar.xz")
            .trim_end_matches(".tar.gz")
            .to_string()
    }
    pub fn part_path(&self) -> PathBuf {
        util::state_dir()
            .join("downloads")
            .join(format!("{}.part", self.name))
    }
}

/// x86-64-v3 (AVX2 등) 지원 여부. 없으면 v3 빌드는 Illegal instruction 으로 죽는다.
pub fn cpu_supports_v3() -> bool {
    let Ok(info) = fs::read_to_string("/proc/cpuinfo") else {
        return false;
    };
    let Some(flags) = info.lines().find(|l| l.starts_with("flags")) else {
        return false;
    };
    let flags: Vec<&str> = flags.split_whitespace().collect();
    ["avx2", "bmi1", "bmi2", "fma", "movbe", "f16c"]
        .iter()
        .all(|f| flags.contains(f))
        && (flags.contains(&"abm") || flags.contains(&"lzcnt"))
}

/// 최신 릴리스에서 이 CPU 에 맞는 파일 고르기
pub fn latest_asset() -> Result<Asset, String> {
    // 테스트용: 다른 릴리스 JSON 주소로 바꿀 수 있음
    let api = std::env::var("KTL_PROTON_RELEASE_API").unwrap_or_else(|_| CACHYOS_API.into());
    let rel = util::fetch_json(&api)?;
    let assets: Vec<(String, String, u64)> = rel["assets"]
        .as_array()
        .ok_or_else(|| {
            format!(
                "GitHub 응답 오류: {}",
                rel["message"].as_str().unwrap_or("?")
            )
        })?
        .iter()
        .filter_map(|a| {
            Some((
                a["name"].as_str()?.to_string(),
                a["browser_download_url"].as_str()?.to_string(),
                a["size"].as_u64().unwrap_or(0),
            ))
        })
        .collect();
    pick_asset(&assets, cpu_supports_v3())
}

fn pick_asset(assets: &[(String, String, u64)], v3: bool) -> Result<Asset, String> {
    let suffix = if v3 {
        "-x86_64_v3.tar.xz"
    } else {
        "-x86_64.tar.xz"
    };
    let mut cands: Vec<&(String, String, u64)> = assets
        .iter()
        .filter(|(n, _, _)| n.ends_with(suffix))
        .collect();
    // 검증된 구성은 SLR(Steam Linux Runtime) 빌드
    cands.sort_by_key(|(n, _, _)| !n.contains("-slr-"));
    let (name, url, size) = cands
        .first()
        .ok_or_else(|| format!("릴리스에서 {suffix} 파일을 찾지 못했습니다"))?;
    let stem = name.trim_end_matches(".tar.xz");
    let sum_url = assets
        .iter()
        .find(|(n, _, _)| *n == format!("{name}.sha512sum") || *n == format!("{stem}.sha512sum"))
        .map(|(_, u, _)| u.clone());
    Ok(Asset {
        name: name.clone(),
        url: url.clone(),
        size: *size,
        sum_url,
    })
}

pub fn install_dir() -> PathBuf {
    util::data_home().join("Steam/compatibilitytools.d")
}

/// 받아서 compatibilitytools.d 에 풀기. 반환: 설치된 폴더 이름
pub fn install_asset(asset: &Asset) -> Result<String, String> {
    let dir = install_dir();
    let name = asset.dir_name();
    if dir.join(&name).join("proton").is_file() {
        return Ok(name);
    }
    let part = asset.part_path();
    let _ = fs::remove_file(&part);
    util::download(&asset.url, &part)?;
    if asset.size > 0 && fs::metadata(&part).map(|m| m.len()).unwrap_or(0) != asset.size {
        let _ = fs::remove_file(&part);
        return Err("다운로드 크기가 맞지 않습니다".into());
    }
    if let Some(sum_url) = &asset.sum_url {
        let sum_file = part.with_extension("sha512sum");
        util::download(sum_url, &sum_file)?;
        let want = fs::read_to_string(&sum_file)
            .unwrap_or_default()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let _ = fs::remove_file(&sum_file);
        let out = std::process::Command::new("sha512sum")
            .arg(&part)
            .output()
            .map_err(|e| e.to_string())?;
        let got = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if want.is_empty() || want != got {
            let _ = fs::remove_file(&part);
            return Err("체크섬(sha512)이 맞지 않습니다".into());
        }
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let st = std::process::Command::new("tar")
        .arg("-xf")
        .arg(&part)
        .arg("-C")
        .arg(&dir)
        .status()
        .map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&part);
    if !st.success() {
        return Err("압축 해제 실패".into());
    }
    if !dir.join(&name).join("proton").is_file() {
        return Err(format!("압축을 풀었지만 {name}/proton 이 없습니다"));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_v3_slr_with_checksum() {
        let a = |n: &str| (n.to_string(), format!("https://x/{n}"), 1u64);
        let assets = vec![
            a("proton-cachyos-11.0-20261014-x86_64_v3.tar.xz"),
            a("proton-cachyos-11.0-20261014-slr-x86_64_v3.tar.xz"),
            a("proton-cachyos-11.0-20261014-slr-x86_64_v3.sha512sum"),
            a("proton-cachyos-11.0-20261014-slr-x86_64.tar.xz"),
        ];
        let v3 = pick_asset(&assets, true).unwrap();
        assert_eq!(v3.dir_name(), "proton-cachyos-11.0-20261014-slr-x86_64_v3");
        assert!(v3.sum_url.is_some());
        let plain = pick_asset(&assets, false).unwrap();
        assert_eq!(plain.dir_name(), "proton-cachyos-11.0-20261014-slr-x86_64");
        assert!(plain.sum_url.is_none());
    }
}
