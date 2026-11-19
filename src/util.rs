//! 경로, 비밀 값 마스킹, 프로세스 조회, 다운로드 등 공용 도우미

use regex::Regex;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

// ───────────────────────── 경로 ─────────────────────────

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn expand(s: &str) -> PathBuf {
    if s == "~" {
        home()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(s)
    }
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if Path::new(&v).is_absolute() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

pub fn data_home() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("kakaotalk-launcher")
}
pub fn data_dir() -> PathBuf {
    data_home().join("kakaotalk-launcher")
}
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("kakaotalk-launcher")
}

pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp-launcher");
    fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn now_string() -> String {
    Command::new("date")
        .arg("+%F %T")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn file_mtime_string(p: &Path) -> String {
    Command::new("date")
        .arg("-r")
        .arg(p)
        .arg("+%F %T")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(bin))
            .find(|p| p.is_file())
    })
}

// ─────────────────────── 비밀 값 마스킹 ───────────────────────

/// 환경 변수 이름이 비밀 값으로 보이는지 (자식 프로세스에 넘기지 않음)
static SECRET_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(KEY|TOKEN|SECRET|PASSWORD|PASSWD|PASSPHRASE|CREDENTIAL|COOKIE|PRIVATE)")
        .unwrap()
});

/// 로그 줄에서 `NAME=value`, `'NAME': 'value'` 형태 마스킹
static SECRET_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\b([A-Za-z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|PASSWD|PASSPHRASE|CREDENTIALS?|COOKIE|PRIVATE)[A-Za-z0-9_]*)(['"]?\s*[=:]\s*['"]?)([^\s'",}]+)"#,
    )
    .unwrap()
});

/// 이름과 상관없이 흔한 토큰 형식
static SECRET_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(sk-[A-Za-z0-9_\-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[abprs]-[A-Za-z0-9\-]{10,}|AIza[0-9A-Za-z_\-]{30,}|AKIA[0-9A-Z]{16}|glpat-[A-Za-z0-9_\-]{20,})",
    )
    .unwrap()
});

pub fn is_secret_name(name: &str) -> bool {
    SECRET_NAME.is_match(name)
}

pub fn mask_secrets(text: &str) -> String {
    let t = SECRET_ASSIGN.replace_all(text, "${1}${2}********");
    SECRET_TOKEN.replace_all(&t, "********").into_owned()
}

// ─────────────────────── 프로세스 조회 ───────────────────────

#[derive(Debug, Clone)]
pub struct Proc {
    pub pid: u32,
    /// argv[0] 의 파일 이름 (소문자, `/` 와 `\\` 모두 구분자로 취급)
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// prefix 관련 프로세스 없음
    Stopped,
    /// umu/Proton/wineserver 는 있으나 KakaoTalkUI.exe 아직 없음
    Starting,
    Running,
    /// CrashReporter.exe 실행 중
    Crashed,
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('/').to_string()
}

/// 해당 WINEPREFIX 로 돌고 있는 프로세스(umu-run, proton, wineserver, exe 등)
pub fn prefix_procs(prefix: &Path) -> Vec<Proc> {
    let want = norm(prefix);
    let me = std::process::id();
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return out;
    };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if pid == me {
            continue;
        }
        let dir = e.path();
        let Ok(cmdline) = fs::read(dir.join("cmdline")) else {
            continue;
        };
        if cmdline.is_empty() {
            continue; // 커널 스레드 / 좀비
        }
        let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or_default();
        let argv0 = String::from_utf8_lossy(argv0).to_ascii_lowercase();
        let name = argv0
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string();
        // 반드시 WINEPREFIX 로 판별한다. 이름(KakaoTalkUI.exe)만 보면 다른 prefix
        // (호스트의 카카오톡, Bottles판 등)까지 잡혀서 강제 종료 시 함께 죽는다.
        // Wine 자식 프로세스는 WINEPREFIX 를 물려받으므로 이것으로 충분하다.
        let mut hit = false;
        if let Ok(env) = fs::read(dir.join("environ")) {
            hit = env.split(|b| *b == 0).any(|kv| {
                let kv = String::from_utf8_lossy(kv);
                for key in ["WINEPREFIX=", "STEAM_COMPAT_DATA_PATH="] {
                    if let Some(v) = kv.strip_prefix(key)
                        && v.trim_end_matches('/') == want
                    {
                        return true;
                    }
                }
                false
            });
        }
        if hit {
            out.push(Proc { pid, name });
        }
    }
    out
}

pub fn run_state(prefix: &Path) -> RunState {
    let procs = prefix_procs(prefix);
    if procs.is_empty() {
        return RunState::Stopped;
    }
    let has = |n: &str| procs.iter().any(|p| p.name == n);
    if has("crashreporter.exe") {
        RunState::Crashed
    } else if has("kakaotalkui.exe") {
        RunState::Running
    } else {
        RunState::Starting
    }
}

pub fn wait_stopped(prefix: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if prefix_procs(prefix).is_empty() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn signal(pids: &[u32], sig: &str) {
    if pids.is_empty() {
        return;
    }
    let _ = Command::new("kill")
        .arg(sig)
        .args(pids.iter().map(|p| p.to_string()))
        .stderr(Stdio::null())
        .status();
}

/// `wineserver -k` → SIGTERM → SIGKILL 순서로 prefix 를 완전히 종료
pub fn kill_prefix(prefix: &Path, proton: Option<&Path>) -> Result<(), String> {
    if prefix_procs(prefix).is_empty() {
        return Ok(());
    }
    if let Some(ws) = proton.and_then(crate::proton::wineserver) {
        let _ = Command::new(ws)
            .arg("-k")
            .env("WINEPREFIX", prefix)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if wait_stopped(prefix, Duration::from_secs(4)) {
            return Ok(());
        }
    }
    let pids: Vec<u32> = prefix_procs(prefix).iter().map(|p| p.pid).collect();
    signal(&pids, "-TERM");
    if wait_stopped(prefix, Duration::from_secs(3)) {
        return Ok(());
    }
    let pids: Vec<u32> = prefix_procs(prefix).iter().map(|p| p.pid).collect();
    signal(&pids, "-KILL");
    if wait_stopped(prefix, Duration::from_secs(2)) {
        Ok(())
    } else {
        Err("일부 프로세스가 종료되지 않았습니다".into())
    }
}

// ───────────────────────── 다운로드 ─────────────────────────

/// curl, 없으면 wget 으로 파일 다운로드
pub fn download(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(p) = dest.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let ua = "kakaotalk-launcher";
    let curl = Command::new("curl")
        .args([
            "-fsSL",
            "--retry",
            "2",
            "--connect-timeout",
            "15",
            "-A",
            ua,
            "-o",
        ])
        .arg(dest)
        .arg(url)
        .status();
    match curl {
        Ok(s) if s.success() => return Ok(()),
        Ok(s) => {
            return Err(format!(
                "다운로드 실패 ({url}): curl 종료 코드 {:?}",
                s.code()
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let wget = Command::new("wget")
        .args(["-q", "-U", ua, "-O"])
        .arg(dest)
        .arg(url)
        .status();
    match wget {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!(
            "다운로드 실패 ({url}): wget 종료 코드 {:?}",
            s.code()
        )),
        Err(_) => Err("curl 또는 wget 이 필요합니다 (sudo apt install curl)".into()),
    }
}

pub fn fetch_json(url: &str) -> Result<serde_json::Value, String> {
    let tmp = state_dir().join(format!("fetch-{}.json", std::process::id()));
    download(url, &tmp)?;
    let s = fs::read_to_string(&tmp).map_err(|e| e.to_string());
    let _ = fs::remove_file(&tmp);
    serde_json::from_str(&s?).map_err(|e| format!("JSON 파싱 실패: {e}"))
}

pub fn notify(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "KakaoTalk", "-i", "kakaotalk", title, body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn open_uri(uri: &str) {
    let _ = Command::new("xdg-open")
        .arg(uri)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn open_path(p: &Path) {
    let _ = Command::new("xdg-open")
        .arg(p)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        let s = "OPENAI_API_KEY=sk-abc123 'GITHUB_TOKEN': 'ghp_x' DXVK_FILTER_DEVICE_NAME=Intel";
        let m = mask_secrets(s);
        assert!(!m.contains("sk-abc123"));
        assert!(!m.contains("ghp_x"));
        assert!(m.contains("DXVK_FILTER_DEVICE_NAME=Intel"));
        assert!(m.contains("OPENAI_API_KEY=********"));
        let t = mask_secrets("token is sk-ant-AAAAAAAAAAAAAAAAAAAAAAAA here");
        assert!(t.contains("token is ******** here"));
    }

    #[test]
    fn secret_names() {
        assert!(is_secret_name("ANTHROPIC_API_KEY"));
        assert!(is_secret_name("HF_TOKEN"));
        assert!(!is_secret_name("XMODIFIERS"));
        assert!(!is_secret_name("WAYLAND_DISPLAY"));
    }
}
