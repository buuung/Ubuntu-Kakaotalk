//! GPU 감지와 DXVK_FILTER_DEVICE_NAME 자동 결정
//!
//! 규칙
//! - 실제 GPU(Intel/AMD/NVIDIA) 제조사가 하나뿐 → 필터 없음
//! - 여러 제조사 → 화면 출력 GPU(boot_vga=1). 노트북(Optimus 등)은 보통 내장 GPU → 배터리 절약
//! - 실제 GPU 없음(가상머신) → 필터 없음 (lavapipe 소프트웨어 Vulkan)
//! - 고른 필터와 맞는 Vulkan 장치가 없으면 필터를 끈다 (맞는 장치가 없으면 DXVK 가 GPU 를 못 찾아 실행 실패)

use crate::util;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

pub const ENV_NAME: &str = "DXVK_FILTER_DEVICE_NAME";
pub const MODE_AUTO: &str = "auto";
pub const MODE_NONE: &str = "none";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    Amd,
    Nvidia,
    /// virtio / QXL / VMware / VirtualBox / QEMU 표준 VGA
    Virtual,
    Other,
}

impl Vendor {
    fn from_id(id: &str) -> Self {
        match id
            .trim()
            .trim_start_matches("0x")
            .to_ascii_lowercase()
            .as_str()
        {
            "8086" => Vendor::Intel,
            "1002" | "1022" => Vendor::Amd,
            "10de" => Vendor::Nvidia,
            "1af4" | "1b36" | "15ad" | "80ee" | "1234" => Vendor::Virtual,
            _ => Vendor::Other,
        }
    }

    /// Vulkan 장치 이름에 들어 있는 단어 (DXVK_FILTER_DEVICE_NAME 값)
    pub fn filter(self) -> Option<&'static str> {
        match self {
            Vendor::Intel => Some("Intel"),
            Vendor::Amd => Some("AMD"),
            Vendor::Nvidia => Some("NVIDIA"),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Vendor::Intel => "Intel",
            Vendor::Amd => "AMD",
            Vendor::Nvidia => "NVIDIA",
            Vendor::Virtual => "가상 GPU",
            Vendor::Other => "기타",
        }
    }

    fn is_real(self) -> bool {
        matches!(self, Vendor::Intel | Vendor::Amd | Vendor::Nvidia)
    }
}

#[derive(Debug, Clone)]
pub struct Gpu {
    pub vendor: Vendor,
    pub pci: String,
    pub driver: String,
    /// 부팅 시 화면 출력 GPU
    pub boot_vga: bool,
}

impl Gpu {
    pub fn describe(&self) -> String {
        let mut s = format!("{} ({})", self.vendor.label(), self.driver);
        if self.boot_vga {
            s += " · 화면 출력";
        }
        s + &format!(" · {}", self.pci)
    }
}

/// /sys/class/drm 의 cardN 장치 목록
pub fn detect() -> Vec<Gpu> {
    detect_in(Path::new("/sys/class/drm"))
}

fn detect_in(drm: &Path) -> Vec<Gpu> {
    let mut out: Vec<Gpu> = Vec::new();
    let mut seen: Vec<std::path::PathBuf> = Vec::new();
    let Ok(rd) = fs::read_dir(drm) else {
        return out;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        // card0, card1 … (card0-eDP-1 같은 커넥터 제외)
        if !name
            .strip_prefix("card")
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        let dev = e.path().join("device");
        let Ok(vendor) = fs::read_to_string(dev.join("vendor")) else {
            continue;
        };
        // 같은 PCI 장치가 여러 card 로 보이는 경우 중복 제거
        let real = fs::canonicalize(&dev).unwrap_or(dev.clone());
        if seen.contains(&real) {
            continue;
        }
        seen.push(real.clone());
        let pci = real
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(name.clone());
        let driver = fs::read_link(dev.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "드라이버 없음".into());
        let boot_vga = fs::read_to_string(dev.join("boot_vga")).is_ok_and(|s| s.trim() == "1");
        out.push(Gpu {
            vendor: Vendor::from_id(&vendor),
            pci,
            driver,
            boot_vga,
        });
    }
    out
}

/// `vulkaninfo --summary` 의 deviceName 목록. vulkaninfo 가 없으면 빈 목록.
pub fn vulkan_devices() -> Vec<String> {
    if util::which("vulkaninfo").is_none() {
        return Vec::new();
    }
    let Ok(out) = Command::new("vulkaninfo")
        .arg("--summary")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    parse_vulkaninfo(&String::from_utf8_lossy(&out.stdout))
}

fn parse_vulkaninfo(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let (k, val) = l.split_once('=')?;
            (k.trim() == "deviceName").then(|| val.trim().to_string())
        })
        .collect();
    v.dedup();
    v
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub filter: Option<String>,
    pub reason: String,
}

pub fn auto_choice(gpus: &[Gpu]) -> Choice {
    let real: Vec<&Gpu> = gpus.iter().filter(|g| g.vendor.is_real()).collect();
    let mut vendors: Vec<Vendor> = real.iter().map(|g| g.vendor).collect();
    vendors.dedup();
    vendors.sort_by_key(|v| v.label());
    vendors.dedup();
    match vendors.len() {
        0 => Choice {
            filter: None,
            reason: "실제 GPU 없음(가상머신 등) — 필터 없이 소프트웨어 Vulkan(llvmpipe) 사용"
                .into(),
        },
        1 => Choice {
            filter: None,
            reason: format!("GPU 제조사가 하나({}) — 필터 불필요", vendors[0].label()),
        },
        _ => {
            // 화면 출력 GPU, 없으면 내장일 가능성이 큰 순서 (Intel → AMD → NVIDIA)
            let pick = real
                .iter()
                .find(|g| g.boot_vga)
                .map(|g| g.vendor)
                .unwrap_or_else(|| {
                    [Vendor::Intel, Vendor::Amd, Vendor::Nvidia]
                        .into_iter()
                        .find(|v| vendors.contains(v))
                        .unwrap()
                });
            Choice {
                filter: pick.filter().map(String::from),
                reason: format!(
                    "GPU 여러 개({}) 중 화면 출력 GPU인 {} 사용 — 노트북이면 내장 GPU라 배터리 절약",
                    vendors
                        .iter()
                        .map(|v| v.label())
                        .collect::<Vec<_>>()
                        .join(" + "),
                    pick.label()
                ),
            }
        }
    }
}

/// 실제로 쓸 필터 결정.
/// `mode`: "auto" | "none" | 직접 지정한 문자열, `env_override`: 추가 환경 변수에 적은 값(최우선)
pub fn resolve(mode: &str, env_override: Option<&str>, gpus: &[Gpu], vk: &[String]) -> Choice {
    let mut c = match (env_override, mode.trim()) {
        (Some(v), _) if !v.trim().is_empty() => Choice {
            filter: Some(v.trim().to_string()),
            reason: format!("추가 환경 변수의 {ENV_NAME} 사용"),
        },
        (_, MODE_NONE) => Choice {
            filter: None,
            reason: "필터 사용 안 함".into(),
        },
        (_, "" | MODE_AUTO) => auto_choice(gpus),
        (_, custom) => Choice {
            filter: Some(custom.to_string()),
            reason: "직접 지정".into(),
        },
    };

    // 안전장치: 맞는 장치가 없으면 필터를 끈다
    if let Some(f) = c.filter.clone() {
        let mismatch = if !vk.is_empty() {
            !vk.iter().any(|n| n.contains(&f))
        } else {
            // vulkaninfo 가 없으면 sysfs 제조사로만 판단 (알려진 단어일 때만)
            let known = [Vendor::Intel, Vendor::Amd, Vendor::Nvidia]
                .into_iter()
                .find(|v| v.filter() == Some(f.as_str()));
            known.is_some_and(|v| !gpus.iter().any(|g| g.vendor == v))
        };
        if mismatch {
            let found = if vk.is_empty() {
                gpus.iter()
                    .map(|g| g.vendor.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                vk.join(", ")
            };
            c.reason = format!(
                "{} — 그런데 '{f}'와 맞는 GPU가 없어서({}) 필터를 껐습니다",
                c.reason,
                if found.is_empty() {
                    "감지된 장치 없음".into()
                } else {
                    found
                }
            );
            c.filter = None;
        }
    }
    c
}

/// 설정 기준으로 한 번에 결정 (실행 직전에 호출)
pub fn resolve_for(cfg: &crate::config::Config) -> Choice {
    resolve(
        &cfg.gpu,
        cfg.env.get(ENV_NAME).map(String::as_str),
        &detect(),
        &vulkan_devices(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(v: Vendor, boot: bool) -> Gpu {
        Gpu {
            vendor: v,
            pci: format!("{v:?}"),
            driver: "x".into(),
            boot_vga: boot,
        }
    }

    #[test]
    fn optimus_laptop_picks_intel() {
        let gpus = [g(Vendor::Intel, true), g(Vendor::Nvidia, false)];
        assert_eq!(auto_choice(&gpus).filter.as_deref(), Some("Intel"));
    }

    #[test]
    fn desktop_dgpu_display_picks_dgpu() {
        let gpus = [g(Vendor::Intel, false), g(Vendor::Amd, true)];
        assert_eq!(auto_choice(&gpus).filter.as_deref(), Some("AMD"));
    }

    #[test]
    fn single_or_vm_no_filter() {
        assert_eq!(auto_choice(&[g(Vendor::Amd, true)]).filter, None);
        assert_eq!(auto_choice(&[g(Vendor::Virtual, true)]).filter, None);
        assert_eq!(auto_choice(&[]).filter, None);
        // Intel 내장 + Intel Arc: 같은 제조사라 필터 의미 없음
        assert_eq!(
            auto_choice(&[g(Vendor::Intel, true), g(Vendor::Intel, false)]).filter,
            None
        );
    }

    #[test]
    fn mismatched_filter_is_dropped() {
        let vk = vec!["llvmpipe (LLVM 19.1.1, 256 bits)".to_string()];
        let c = resolve(MODE_AUTO, Some("Intel"), &[g(Vendor::Virtual, true)], &vk);
        assert_eq!(c.filter, None);
        let gpus = [g(Vendor::Amd, true)];
        assert_eq!(resolve("Intel", None, &gpus, &[]).filter, None);
        let vk = vec![
            "Intel(R) Iris(R) Xe Graphics (TGL GT2)".to_string(),
            "NVIDIA GeForce MX450".to_string(),
        ];
        let gpus = [g(Vendor::Intel, true), g(Vendor::Nvidia, false)];
        assert_eq!(
            resolve(MODE_AUTO, None, &gpus, &vk).filter.as_deref(),
            Some("Intel")
        );
        assert_eq!(
            resolve("NVIDIA", None, &gpus, &vk).filter.as_deref(),
            Some("NVIDIA")
        );
    }

    #[test]
    fn parses_vulkaninfo() {
        let s = "GPU0:\n\tdeviceName         = Intel(R) Iris(R) Xe Graphics\nGPU1:\n\tdeviceName = llvmpipe (LLVM)\n";
        assert_eq!(
            parse_vulkaninfo(s),
            vec!["Intel(R) Iris(R) Xe Graphics", "llvmpipe (LLVM)"]
        );
    }

    #[test]
    fn reads_sysfs_layout() {
        let base = std::env::temp_dir().join(format!("ktl-drm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for (card, vendor, boot) in [("card0", "0x8086", "1"), ("card1", "0x10de", "0")] {
            let d = base.join(card).join("device");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("vendor"), vendor).unwrap();
            fs::write(d.join("boot_vga"), boot).unwrap();
        }
        fs::create_dir_all(base.join("card0-eDP-1")).unwrap();
        let gpus = detect_in(&base);
        let _ = fs::remove_dir_all(&base);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].vendor, Vendor::Intel);
        assert!(gpus[0].boot_vga);
        assert_eq!(auto_choice(&gpus).filter.as_deref(), Some("Intel"));
    }
}
