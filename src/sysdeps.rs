//! 실행 전에 확인할 시스템 구성 (Lutris 없이 처음 설치하는 사용자용)

use std::fs;
use std::path::{Path, PathBuf};

// ───────────────────────── 32비트 Vulkan ─────────────────────────

/// 32비트 Vulkan 로더/드라이버가 없으면 Proton 이 "준비 중"에서 멈출 수 있다 (Lutris 무한로딩과 같은 원인)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vulkan32 {
    pub loader: bool,
    pub driver: bool,
}

impl Vulkan32 {
    pub fn ok(&self) -> bool {
        self.loader && self.driver
    }

    pub fn summary(&self) -> &'static str {
        match (self.loader, self.driver) {
            (true, true) => "설치됨",
            (false, _) => {
                "32비트 Vulkan 로더(libvulkan1:i386) 없음 — 카카오톡이 '준비 중'에서 멈출 수 있음"
            }
            (true, false) => "32비트 Vulkan 드라이버 없음 — 카카오톡이 '준비 중'에서 멈출 수 있음",
        }
    }
}

pub const VULKAN32_CMD: &str = "sudo dpkg --add-architecture i386 && sudo apt update && sudo apt install -y libvulkan1:i386 mesa-vulkan-drivers:i386";

const LIB32_DIRS: [&str; 2] = ["usr/lib/i386-linux-gnu", "usr/lib32"];

pub fn vulkan32() -> Vulkan32 {
    vulkan32_in(Path::new("/"))
}

fn vulkan32_in(root: &Path) -> Vulkan32 {
    let lib32 = |name: &str| LIB32_DIRS.iter().any(|d| root.join(d).join(name).exists());
    let loader = lib32("libvulkan.so.1");
    // 드라이버는 ICD JSON 이름으로 판별하면 안 된다: Ubuntu/Debian 은 intel_icd.json 처럼
    // 아키텍처 표시 없이 32/64비트가 같은 JSON 을 공유한다. 대신 32비트 드라이버 .so 를 찾는다.
    //   Mesa: libvulkan_intel.so, libvulkan_radeon.so, libvulkan_lvp.so …
    //   NVIDIA: libGLX_nvidia.so.0 (32비트 libnvidia-gl)
    let mesa = LIB32_DIRS.iter().any(|d| {
        fs::read_dir(root.join(d))
            .map(|rd| {
                rd.flatten().any(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.starts_with("libvulkan_") && n.contains(".so")
                })
            })
            .unwrap_or(false)
    });
    // 일부 배포판(Fedora 등)은 *.i686.json 으로 구분
    let icd_i686 = fs::read_dir(root.join("usr/share/vulkan/icd.d"))
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().contains("i686"))
        })
        .unwrap_or(false);
    let nvidia = lib32("libGLX_nvidia.so.0");
    Vulkan32 {
        loader,
        driver: mesa || icd_i686 || nvidia,
    }
}

// ─────────────── AppArmor: 비특권 사용자 네임스페이스 제한 ───────────────
//
// Ubuntu 24.04+ 는 kernel.apparmor_restrict_unprivileged_userns=1 로, AppArmor 프로필이 없는
// 프로그램이 사용자 네임스페이스를 쓰지 못하게 한다. umu 의 Steam Runtime(pressure-vessel)은
// bwrap 으로 컨테이너를 만들기 때문에 "bwrap: setting up uid map: Permission denied" 로 실패한다.
// Steam Runtime 경로에만 userns 를 허용하는 프로필을 설치하면 시스템 전체 제한은 유지된다.

pub const APPARMOR_PROFILE_PATH: &str = "/etc/apparmor.d/kakaotalk-launcher-umu";
const SYSCTL: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_userns";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Userns {
    /// 제한 없음 (Ubuntu 23.10 이전, 다른 배포판, 또는 sysctl=0)
    Unrestricted,
    /// 제한은 켜져 있지만 이 런처의 허용 프로필이 설치됨
    Allowed,
    /// 제한이 켜져 있고 허용 프로필 없음 → Proton 실행 실패
    Blocked,
}

impl Userns {
    pub fn ok(self) -> bool {
        self != Userns::Blocked
    }

    pub fn summary(self) -> &'static str {
        match self {
            Userns::Unrestricted => "제한 없음",
            Userns::Allowed => "Steam Runtime 허용 프로필 설치됨",
            Userns::Blocked => {
                "Ubuntu 보안 설정이 Steam Runtime 컨테이너를 막고 있음 — 허용 프로필 설치 필요"
            }
        }
    }
}

pub fn userns() -> Userns {
    userns_with(Path::new(SYSCTL), Path::new(APPARMOR_PROFILE_PATH))
}

fn userns_with(sysctl: &Path, profile: &Path) -> Userns {
    let restricted = fs::read_to_string(sysctl).is_ok_and(|s| s.trim() == "1");
    if !restricted {
        Userns::Unrestricted
    } else if profile.is_file() {
        Userns::Allowed
    } else {
        Userns::Blocked
    }
}

/// umu 가 Steam Runtime 을 두는 곳 (~/.local/share/umu)
fn umu_runtime_dir() -> PathBuf {
    crate::util::data_home().join("umu")
}

/// Steam Runtime(pressure-vessel) 경로에만 userns 를 허용하는 AppArmor 프로필.
/// Ubuntu 가 steam 등에 기본 제공하는 프로필과 같은 형식 (unconfined + userns).
pub fn apparmor_profile() -> String {
    let dir = umu_runtime_dir();
    format!(
        "# kakaotalk-launcher: umu Steam Runtime(pressure-vessel) 에 사용자 네임스페이스 허용
# Ubuntu 24.04+ 의 kernel.apparmor_restrict_unprivileged_userns=1 대응
abi <abi/4.0>,
include <tunables/global>

profile kakaotalk-launcher-umu \"{}/steamrt*/**\" flags=(unconfined) {{
  userns,

  include if exists <local/kakaotalk-launcher-umu>
}}
",
        dir.display()
    )
}

/// 터미널에 붙여넣을 한 줄 명령 (런처가 프로필을 출력 → sudo 로 설치 → 적용)
pub fn apparmor_install_cmd() -> String {
    let me = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "kakaotalk-proton".into());
    format!(
        "'{me}' --apparmor-profile | sudo tee {APPARMOR_PROFILE_PATH} >/dev/null && sudo apparmor_parser -r {APPARMOR_PROFILE_PATH}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("ktl-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        r
    }

    #[test]
    fn detects_ubuntu_layout() {
        let r = tmp("v32");
        assert!(!vulkan32_in(&r).ok());
        let lib = r.join("usr/lib/i386-linux-gnu");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("libvulkan.so.1"), "").unwrap();
        assert_eq!(
            vulkan32_in(&r),
            Vulkan32 {
                loader: true,
                driver: false
            }
        );
        // Ubuntu: ICD JSON 은 arch 표시 없음 → .so 로 판별
        fs::create_dir_all(r.join("usr/share/vulkan/icd.d")).unwrap();
        fs::write(r.join("usr/share/vulkan/icd.d/lvp_icd.json"), "").unwrap();
        assert!(!vulkan32_in(&r).driver);
        fs::write(lib.join("libvulkan_lvp.so"), "").unwrap();
        assert!(vulkan32_in(&r).ok());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn detects_fedora_and_nvidia() {
        let r = tmp("v32b");
        fs::create_dir_all(r.join("usr/lib32")).unwrap();
        fs::write(r.join("usr/lib32/libvulkan.so.1"), "").unwrap();
        fs::write(r.join("usr/lib32/libGLX_nvidia.so.0"), "").unwrap();
        assert!(vulkan32_in(&r).ok());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn userns_states() {
        let r = tmp("userns");
        fs::create_dir_all(&r).unwrap();
        let sysctl = r.join("sysctl");
        let profile = r.join("profile");
        assert_eq!(userns_with(&sysctl, &profile), Userns::Unrestricted);
        fs::write(&sysctl, "0\n").unwrap();
        assert_eq!(userns_with(&sysctl, &profile), Userns::Unrestricted);
        fs::write(&sysctl, "1\n").unwrap();
        assert_eq!(userns_with(&sysctl, &profile), Userns::Blocked);
        fs::write(&profile, "x").unwrap();
        assert_eq!(userns_with(&sysctl, &profile), Userns::Allowed);
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn profile_mentions_runtime() {
        let p = apparmor_profile();
        assert!(p.contains("userns,"));
        assert!(p.contains("/umu/steamrt*/**"));
        assert!(p.contains("flags=(unconfined)"));
    }
}
