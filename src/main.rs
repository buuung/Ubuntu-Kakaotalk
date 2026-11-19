//! 카카오톡 전용 Proton 런처
//!
//!   kakaotalk-proton              설정 창
//!   kakaotalk-proton --launch     UI 없이 바로 실행 (앱 메뉴 바로가기)
//!   kakaotalk-proton --install F  사용자가 받은 설치 파일 실행
//!   kakaotalk-proton --get-proton 최신 Proton-CachyOS 받기
//!   kakaotalk-proton --kill       강제 종료 (wineserver -k)
//!   kakaotalk-proton --repair     레지스트리 수리 (카카오톡 꺼져 있을 때)
//!   kakaotalk-proton --status     상태 출력
//!   kakaotalk-proton --install-desktop   앱 메뉴 바로가기 교체

mod config;
mod desktop;
mod fonts;
mod gpu;
mod icon;
mod proton;
mod registry;
mod runner;
mod sysdeps;
mod ui;
mod umu;
mod util;

use config::Config;
use std::process::ExitCode;

/// stdout 이 닫혀도(| head) 패닉하지 않는 println
macro_rules! say {
    ($($t:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}

const HELP: &str = "kakaotalk-proton — KakaoTalk(Windows) Proton 런처

사용법:
  kakaotalk-proton                    설정 창 열기
  kakaotalk-proton --settings         설정 창(설정 탭) 열기
  kakaotalk-proton --page <탭>        home | settings | repair | logs 탭으로 열기
  kakaotalk-proton --launch           UI 없이 바로 실행 (첫 실행 준비가 필요하면 진행 창 표시)
  kakaotalk-proton --install <설치파일> 공식 사이트에서 받은 KakaoTalk_Setup.exe 를 Proton 으로 실행
  kakaotalk-proton --get-proton       최신 Proton-CachyOS 받기 (CPU 에 맞는 v3 / 일반 빌드)
  kakaotalk-proton --kill             카카오톡 prefix 강제 종료
  kakaotalk-proton --apparmor-profile Ubuntu 24.04+ 용 Steam Runtime 허용 AppArmor 프로필 출력
  kakaotalk-proton --repair           Windows 10 기본값 / InputStyle 레지스트리 수리
  kakaotalk-proton --status           실행 상태와 점검 결과 출력
  kakaotalk-proton --install-desktop  ~/.local/share/applications/kakaotalk.desktop 교체
";

fn fail(msg: &str) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);

    if has("--help") || has("-h") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }

    if has("--launch") {
        let mut cfg = Config::load();
        if !has("--no-gui") && runner::needs_setup(&cfg) {
            // 첫 실행: umu 가 Steam Runtime/Proton 을 내려받는 동안 진행 상황 표시
            return ui::run(true, None);
        }
        return match runner::run_blocking(&mut cfg) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                util::notify("카카오톡 실행 실패", &e);
                fail(&e)
            }
        };
    }

    if let Some(i) = args.iter().position(|a| a == "--install") {
        let Some(path) = args.get(i + 1) else {
            return fail("사용법: kakaotalk-proton --install <KakaoTalk_Setup.exe>");
        };
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
        let mut cfg = Config::load();
        return match runner::run_installer(&mut cfg, &path) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                util::notify("카카오톡 설치 실패", &e);
                fail(&e)
            }
        };
    }

    if has("--get-proton") {
        let r = proton::latest_asset().and_then(|a| {
            say!("받는 중: {} ({} MB)", a.name, a.size / 1_000_000);
            proton::install_asset(&a)
        });
        return match r {
            Ok(name) => {
                let mut cfg = Config::load();
                if cfg.proton_path().is_none() {
                    cfg.proton = name.clone();
                    let _ = cfg.save();
                }
                say!("설치됨: {name}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }

    if has("--apparmor-profile") {
        print!("{}", sysdeps::apparmor_profile());
        return ExitCode::SUCCESS;
    }

    if has("--kill") {
        let cfg = Config::load();
        return match util::kill_prefix(&cfg.prefix_path(), cfg.proton_path().as_deref()) {
            Ok(()) => {
                say!("종료됨");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }

    if has("--repair") {
        let cfg = Config::load();
        return match registry::apply(&cfg.prefix_path(), &registry::FIXES) {
            Ok(n) => {
                say!("레지스트리 {n}개 항목 수정");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }

    if has("--status") {
        let cfg = Config::load();
        let prefix = cfg.prefix_path();
        say!("상태      : {:?}", util::run_state(&prefix));
        say!("prefix    : {}", prefix.display());
        say!(
            "exe       : {} ({})",
            cfg.exe_path().display(),
            if cfg.exe_path().is_file() {
                "있음"
            } else {
                "없음"
            }
        );
        say!("Proton    : {} → {:?}", cfg.proton, cfg.proton_path());
        say!("umu-run   : {:?}", umu::resolve(&cfg));
        say!(
            "SteamRT   : {}",
            if umu::steam_runtime_present() {
                "있음"
            } else {
                "없음 (첫 실행 시 다운로드)"
            }
        );
        match registry::check(&prefix) {
            Ok(v) => {
                for (f, ok) in v {
                    say!("{:<10}: {}", f.id, if ok { "OK" } else { "필요" });
                }
            }
            Err(e) => say!("레지스트리: {e}"),
        }
        say!("Win 빌드  : {:?}", registry::windows_build(&prefix));
        say!("Vulkan32  : {}", sysdeps::vulkan32().summary());
        say!("AppArmor  : {}", sysdeps::userns().summary());
        for g in gpu::detect() {
            say!("GPU       : {}", g.describe());
        }
        for n in gpu::vulkan_devices() {
            say!("Vulkan    : {n}");
        }
        let c = gpu::resolve_for(&cfg);
        say!(
            "DXVK 필터 : {} — {}",
            c.filter.as_deref().unwrap_or("없음"),
            c.reason
        );
        say!("폰트      : {:?}", fonts::installed(&cfg.font_file));
        say!("바로가기  : {:?}", desktop::current_exec());
        return ExitCode::SUCCESS;
    }

    if has("--install-desktop") {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return fail(&e.to_string()),
        };
        return match desktop::install(&exe) {
            Ok(p) => {
                say!("{} 갱신됨", p.display());
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        };
    }

    let page = if has("--settings") {
        Some("settings".to_string())
    } else {
        args.iter()
            .position(|a| a == "--page")
            .and_then(|i| args.get(i + 1))
            .filter(|p| ["home", "settings", "repair", "logs"].contains(&p.as_str()))
            .cloned()
    };
    ui::run(false, page)
}
