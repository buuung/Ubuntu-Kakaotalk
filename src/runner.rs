//! 카카오톡 실행 (umu-run + Proton), 로그 기록, 크래시 분석

use crate::config::Config;
use crate::{gpu, registry, umu, util};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const END_MARKER: &str = "=== 런처: 카카오톡 종료";

pub fn log_path() -> PathBuf {
    util::state_dir().join("last.log")
}
pub fn prev_log_path() -> PathBuf {
    util::state_dir().join("previous.log")
}

/// 실행 전에 첫 실행 준비(다운로드)가 필요한지 — 필요하면 진행 상황을 보여줄 GUI 로 실행
pub fn needs_setup(cfg: &Config) -> bool {
    !umu::steam_runtime_present()
        || umu::resolve(cfg).is_none()
        || cfg.proton_path().is_none()
        || !cfg.exe_path().is_file()
        || !crate::sysdeps::vulkan32().ok()
        || !crate::sysdeps::userns().ok()
}

/// 자식 프로세스에 넘길 환경 변수 (로그 헤더 / 미리보기용)
pub fn launch_env(
    cfg: &Config,
    proton: &std::path::Path,
    gpu: &gpu::Choice,
) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = vec![
        (
            "WINEPREFIX".into(),
            cfg.prefix_path().to_string_lossy().into(),
        ),
        ("GAMEID".into(), cfg.game_id.trim().to_string()),
        ("PROTONPATH".into(), proton.to_string_lossy().into()),
    ];
    if cfg.debug_log {
        v.push(("PROTON_LOG".into(), "1".into()));
        v.push((
            "PROTON_LOG_DIR".into(),
            util::state_dir().to_string_lossy().into(),
        ));
        v.push(("UMU_LOG".into(), "1".into()));
    } else if !cfg.env.contains_key("WINEDEBUG") {
        v.push(("WINEDEBUG".into(), "-all".into()));
    }
    if let Some(f) = &gpu.filter {
        v.push((gpu::ENV_NAME.into(), f.clone()));
    }
    for (k, val) in &cfg.env {
        if !matches!(
            k.as_str(),
            "WINEPREFIX" | "GAMEID" | "PROTONPATH" | gpu::ENV_NAME
        ) {
            v.push((k.clone(), val.clone()));
        }
    }
    v
}

fn build_command(
    cfg: &Config,
    gpu: &gpu::Choice,
    target: &std::path::Path,
) -> Result<(Command, Vec<(String, String)>), String> {
    let umu = umu::resolve(cfg).ok_or(
        "umu-run 을 찾을 수 없습니다. 설정에서 'Lutris에서 복사' 또는 'GitHub에서 받기'를 하세요.",
    )?;
    let proton = cfg.proton_path().ok_or_else(|| {
        format!(
            "Proton '{}' 이(가) compatibilitytools.d 에 없습니다. 설정에서 다른 버전을 선택하세요.",
            cfg.proton
        )
    })?;
    let exe = target.to_path_buf();
    if !exe.is_file() {
        return Err(format!("실행 파일이 없습니다: {}", exe.display()));
    }

    let env = launch_env(cfg, &proton, gpu);
    let mut c = Command::new(&umu);
    c.arg(&exe);
    if let Some(dir) = exe.parent() {
        c.current_dir(dir);
    }
    // 사용자 셸의 비밀 값(API 키 등)은 넘기지 않는다 → 로그에도 남지 않음
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if util::is_secret_name(&k) && !cfg.env.contains_key(&*k) {
            c.env_remove(&*k);
        }
    }
    // WineD3D 강제는 Qt6(D3D11) 크래시 원인. 사용자가 명시하지 않았으면 제거
    if !cfg.env.contains_key("PROTON_USE_WINED3D") {
        c.env_remove("PROTON_USE_WINED3D");
    }
    // Wine Wayland 드라이버는 한글 입력이 안 됨(검증). 사용자가 명시하지 않았으면 제거
    if !cfg.env.contains_key("PROTON_ENABLE_WAYLAND") {
        c.env_remove("PROTON_ENABLE_WAYLAND");
    }
    // 셸에 남아 있는 필터 값이 자동 결정을 덮어쓰지 않도록
    c.env_remove(gpu::ENV_NAME);
    if !cfg.debug_log {
        c.env_remove("PROTON_LOG").env_remove("UMU_LOG");
    }
    for (k, v) in &env {
        c.env(k, v);
    }
    Ok((c, env))
}

/// UI 없이 실행하고 종료될 때까지 기다린다 (`--launch`)
pub fn run_blocking(cfg: &mut Config) -> Result<i32, String> {
    run_target(cfg, None)
}

/// 사용자가 받은 카카오톡 설치 파일을 같은 prefix / Proton 으로 실행 (`--install`)
/// 설치 파일은 받거나 배포하지 않는다. 사용자가 공식 사이트에서 직접 받은 파일만 실행.
pub fn run_installer(cfg: &mut Config, setup: &std::path::Path) -> Result<i32, String> {
    if !setup.is_file() {
        return Err(format!("설치 파일이 없습니다: {}", setup.display()));
    }
    let _ = fs::create_dir_all(
        cfg.prefix_path()
            .parent()
            .unwrap_or(std::path::Path::new("/")),
    );
    let code = run_target(cfg, Some(setup))?;
    let prefix = cfg.prefix_path();
    // 설치 프로그램이 끝나도 wineserver 가 잠깐 남아 있음 → 꺼진 뒤 레지스트리 수리
    util::wait_stopped(&prefix, std::time::Duration::from_secs(15));
    let mut msg = Vec::new();
    if prefix.join("user.reg").is_file() && util::prefix_procs(&prefix).is_empty() {
        match registry::apply(&prefix, &registry::FIXES) {
            Ok(n) if n > 0 => msg.push(format!("레지스트리 {n}개 항목 수리")),
            Ok(_) => {}
            Err(e) => msg.push(e),
        }
    }
    if cfg.exe_path().is_file() {
        match crate::icon::ensure_icon(&cfg.exe_path()) {
            Ok(_) => {
                // 바로가기가 이 런처를 가리키면 아이콘(Icon=)도 새로 반영
                if let Ok(me) = std::env::current_exe()
                    && crate::desktop::points_to(&me)
                {
                    let _ = crate::desktop::install(&me);
                }
            }
            Err(e) => msg.push(format!("아이콘: {e}")),
        }
        let _ = append_log(&format!("note: 설치 확인됨 {}\n", cfg.exe_path().display()));
        util::notify("카카오톡 설치 완료", &msg.join(" · "));
    } else {
        let _ = append_log("note: 설치 후에도 KakaoTalkUI.exe 를 찾지 못했습니다\n");
        util::notify(
            "카카오톡 설치가 끝나지 않았습니다",
            "설치 창을 끝까지 진행했는지, 설정의 실행 파일 경로가 맞는지 확인하세요",
        );
    }
    Ok(code)
}

fn append_log(text: &str) -> std::io::Result<()> {
    OpenOptions::new()
        .append(true)
        .open(log_path())?
        .write_all(text.as_bytes())
}

fn run_target(cfg: &mut Config, setup: Option<&std::path::Path>) -> Result<i32, String> {
    let prefix = cfg.prefix_path();
    match util::run_state(&prefix) {
        util::RunState::Stopped => {}
        util::RunState::Crashed => {
            return Err(
                "이전 실행이 크래시 상태입니다. 런처 설정에서 강제 종료 후 다시 실행하세요.".into(),
            );
        }
        _ => return Err("카카오톡이 이미 실행 중입니다".into()),
    }

    // Lutris 를 지워도 동작하도록 umu-run 을 자체 경로로 확보
    if cfg.umu_run.trim().is_empty()
        && let Err(e) = umu::ensure_own_copy()
    {
        eprintln!("umu-run 복사 실패: {e}");
    }

    let mut notes: Vec<String> = Vec::new();
    if cfg.last_proton != cfg.proton && !cfg.last_proton.is_empty() {
        notes.push(format!(
            "Proton 변경 감지: {} → {}",
            cfg.last_proton, cfg.proton
        ));
    }
    // 레지스트리 점검 (지금은 prefix 가 꺼져 있으므로 안전)
    if cfg.auto_repair && prefix.join("user.reg").is_file() {
        match registry::apply(&prefix, &registry::FIXES) {
            Ok(0) => {}
            Ok(n) => notes.push(format!(
                "레지스트리 자동 수리: {n}개 항목 (Windows 10 / InputStyle)"
            )),
            Err(e) => notes.push(format!("레지스트리 점검 실패: {e}")),
        }
    }

    let gpu_choice = gpu::resolve_for(cfg);
    notes.push(format!(
        "GPU: {} ({})",
        gpu_choice.filter.as_deref().unwrap_or("필터 없음"),
        gpu_choice.reason
    ));
    let target = setup
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cfg.exe_path());
    let (mut cmd, env) = build_command(cfg, &gpu_choice, &target)?;

    fs::create_dir_all(util::state_dir()).map_err(|e| e.to_string())?;
    if log_path().exists() {
        let _ = fs::rename(log_path(), prev_log_path());
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| e.to_string())?;
    let what = if setup.is_some() {
        "설치 프로그램 실행"
    } else {
        "실행"
    };
    let mut head = format!("=== 런처: {what} {} ===\n", util::now_string());
    head += &format!("exe: {}\n", target.display());
    for (k, v) in &env {
        head += &format!("env: {k}={v}\n");
    }
    for n in &notes {
        head += &format!("note: {n}\n");
    }
    head += "\n";
    let _ = log.write_all(util::mask_secrets(&head).as_bytes());

    let out = log.try_clone().map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    cmd.stdin(Stdio::null()).stdout(out).stderr(err);
    let mut child = cmd.spawn().map_err(|e| format!("umu-run 실행 실패: {e}"))?;

    cfg.last_proton = cfg.proton.clone();
    let _ = cfg.save();

    let status = child.wait().map_err(|e| e.to_string())?;
    let code = status.code().unwrap_or(-1);
    drop(log);

    // 마스킹된 사본으로 교체 + 분석 결과 꼬리말
    let raw = fs::read(log_path()).unwrap_or_default();
    let text = util::mask_secrets(&String::from_utf8_lossy(&raw));
    let findings = analyze(&text);
    let mut tail = format!("\n{END_MARKER} (코드 {code}) {} ===\n", util::now_string());
    for f in &findings {
        tail += &format!("finding: [{}] {}\n", f.level.label(), f.title);
    }
    let _ = util::write_atomic(&log_path(), (text + &tail).as_bytes());

    if let Some(f) = findings.iter().find(|f| f.level == Level::Error)
        && setup.is_none()
    {
        util::notify(&format!("카카오톡 문제 감지: {}", f.title), &f.detail);
    }
    Ok(code)
}

/// GUI 에서 실행: 자기 자신을 `--launch` 로 분리 실행 (GUI 를 닫아도 카카오톡 유지)
pub fn spawn_detached() -> Result<(), String> {
    spawn_self(&["--launch".as_ref(), "--no-gui".as_ref()])
}

pub fn spawn_installer(setup: &std::path::Path) -> Result<(), String> {
    spawn_self(&["--install".as_ref(), setup.as_os_str(), "--no-gui".as_ref()])
}

fn spawn_self(args: &[&std::ffi::OsStr]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(me)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ───────────────────────── 로그 분석 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Info,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Error => "오류",
            Level::Warning => "경고",
            Level::Info => "참고",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

pub fn analyze(text: &str) -> Vec<Finding> {
    let t = text.to_ascii_lowercase();
    let mut v = Vec::new();
    let mut add = |level, title: &str, detail: &str| {
        v.push(Finding {
            level,
            title: title.into(),
            detail: detail.into(),
        })
    };
    let crash = t.contains("crashreporter");
    let h264 = t.contains("h264-used");

    if crash && h264 {
        add(
            Level::Error,
            "동영상(H.264) 처리 중 크래시",
            "현재 Proton의 FFmpeg에 H.264 디코더가 없는 것으로 보입니다(GE-Proton 등). 설정에서 Proton-CachyOS를 선택하세요.",
        );
    } else if crash {
        add(
            Level::Error,
            "카카오톡이 비정상 종료됨 (CrashReporter 실행)",
            "로그 끝부분의 err: 줄을 확인하세요. 디버그 로그(PROTON_LOG=1)를 켜고 재현하면 원인을 찾기 쉽습니다.",
        );
    } else if h264 {
        add(
            Level::Info,
            "H.264 동영상 디코딩 사용",
            "크래시가 없었다면 정상입니다.",
        );
    }
    if t.contains("proton_use_wined3d=1") || t.contains("wined3d_") && t.contains("err:") {
        add(
            Level::Warning,
            "DXVK 대신 WineD3D 사용",
            "카카오톡(Qt6)은 Direct3D11 렌더링이 필요합니다. PROTON_USE_WINED3D 를 지우세요.",
        );
    }
    if t.contains("vkcreateinstance") && t.contains("fail")
        || t.contains("dxvk: no adapters")
        || t.contains("failed to create vulkan")
    {
        add(
            Level::Error,
            "Vulkan 초기화 실패",
            "32비트 Vulkan(libvulkan1:i386, mesa-vulkan-drivers:i386) 설치와 DXVK_FILTER_DEVICE_NAME 값을 확인하세요.",
        );
    }
    if t.contains("setting up uid map: permission denied")
        || (t.contains("bwrap") && t.contains("operation not permitted"))
    {
        add(
            Level::Error,
            "Ubuntu 보안 설정(AppArmor)이 Steam Runtime을 막음",
            "Ubuntu 24.04+ 는 사용자 네임스페이스를 제한합니다. 실행 탭 '처음 설정'의 AppArmor 단계에서 허용 프로필 설치 명령을 복사해 실행하세요.",
        );
    }
    if t.contains("unhandled exception") || t.contains("unhandled page fault") {
        add(
            Level::Warning,
            "처리되지 않은 예외 발생",
            "로그의 해당 줄 주변을 확인하세요.",
        );
    }
    if t.contains("dwmextendframeintoclientarea") {
        add(
            Level::Info,
            "DWM 테두리 API 호출됨",
            "창 양옆 흰 줄이 보이면 '수리 → Windows 10 지정'을 적용하세요.",
        );
    }
    if t.contains("imesetcompositionstring") {
        add(
            Level::Info,
            "IME 조합 API 호출됨",
            "한글 입력이 이상하면 '수리 → InputStyle'이 적용됐는지 확인하세요.",
        );
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_h264_crash() {
        let f = analyze(
            "err:dmo:create_placeholder_file Creating tag file h264-used\n... CrashReporter.exe",
        );
        assert_eq!(f[0].level, Level::Error);
        assert!(f[0].title.contains("H.264"));
    }
}
