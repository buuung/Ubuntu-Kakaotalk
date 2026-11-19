//! 설정 파일 (~/.config/kakaotalk-launcher/config.toml)

use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// 검증된 기본 Proton 버전
pub const DEFAULT_PROTON: &str = "proton-cachyos-11.0-20261014-slr-x86_64_v3";
pub const DEFAULT_PREFIX: &str = "~/Games/kakao-proton";
/// prefix 기준 상대 경로
pub const DEFAULT_EXE_REL: &str =
    "drive_c/users/steamuser/AppData/Local/Programs/Kakao/KakaoTalk/bin/KakaoTalkUI.exe";
pub const DEFAULT_FONT: &str = "Pretendard-Regular-winemoji.ttf";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// WINEPREFIX (`~` 사용 가능)
    pub wineprefix: String,
    /// 실행 파일. 상대 경로면 prefix 기준
    pub exe: String,
    /// compatibilitytools.d 안의 폴더 이름 또는 절대 경로
    pub proton: String,
    /// umu-run 경로. 비우면 자동 탐색
    pub umu_run: String,
    pub game_id: String,
    /// DXVK 가 쓸 GPU: "auto"(감지) | "none"(필터 없음) | Vulkan 장치 이름 일부 (예: "Intel")
    pub gpu: String,
    /// 추가 환경 변수 (WINEPREFIX / GAMEID / PROTONPATH 는 위 항목이 우선)
    pub env: BTreeMap<String, String>,
    /// PROTON_LOG=1 + UMU_LOG=1 (평소엔 WINEDEBUG=-all)
    pub debug_log: bool,
    /// 실행 직전 레지스트리(win10 / InputStyle) 점검 후 자동 수리
    pub auto_repair: bool,
    /// Winemoji 폰트 파일 이름
    pub font_file: String,
    /// 마지막으로 실행에 쓴 Proton (버전 교체 감지용)
    pub last_proton: String,
}

/// 추가 환경 변수 기본값. GPU 선택(DXVK_FILTER_DEVICE_NAME)은 `gpu` 항목이 자동으로 정한다.
pub fn default_env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wineprefix: DEFAULT_PREFIX.into(),
            exe: DEFAULT_EXE_REL.into(),
            proton: DEFAULT_PROTON.into(),
            umu_run: String::new(),
            game_id: "umu-default".into(),
            gpu: crate::gpu::MODE_AUTO.into(),
            env: default_env(),
            debug_log: false,
            auto_repair: true,
            font_file: DEFAULT_FONT.into(),
            last_proton: String::new(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        util::config_dir().join("config.toml")
    }

    pub fn load() -> Self {
        let p = Self::path();
        match fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                eprintln!("설정 파일 오류({}), 기본값 사용: {e}", p.display());
                Self::default()
            }),
            Err(_) => {
                let c = Self::default();
                let _ = c.save();
                c
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let p = Self::path();
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        let s = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        util::write_atomic(&p, s.as_bytes())
    }

    pub fn prefix_path(&self) -> PathBuf {
        util::expand(self.wineprefix.trim())
    }

    pub fn exe_path(&self) -> PathBuf {
        let e = self.exe.trim();
        if e.starts_with('/') || e.starts_with('~') {
            util::expand(e)
        } else {
            self.prefix_path().join(e)
        }
    }

    pub fn proton_path(&self) -> Option<PathBuf> {
        crate::proton::resolve(self.proton.trim())
    }

    /// 환경 변수 편집기용 텍스트
    pub fn env_text(&self) -> String {
        self.env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// "KEY=VALUE" 줄 단위 파싱. 잘못된 줄 번호를 같이 돌려준다.
pub fn parse_env_text(text: &str) -> (BTreeMap<String, String>, Vec<usize>) {
    let mut map = BTreeMap::new();
    let mut bad = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let l = l.strip_prefix("export ").unwrap_or(l);
        match l.split_once('=') {
            Some((k, v))
                if !k.trim().is_empty()
                    && k.trim()
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                let v = v.trim();
                let v = v
                    .strip_prefix('"')
                    .and_then(|x| x.strip_suffix('"'))
                    .unwrap_or(v);
                map.insert(k.trim().to_string(), v.to_string());
            }
            _ => bad.push(i + 1),
        }
    }
    (map, bad)
}
