//! prefix 레지스트리(user.reg) 점검/수정. 반드시 wineserver 가 꺼진 상태에서만 쓴다.

use std::fs;
use std::path::Path;

pub struct Fix {
    pub id: &'static str,
    pub key: &'static str,
    pub name: &'static str,
    pub value: &'static str,
    pub title: &'static str,
    pub why: &'static str,
}

/// prefix 전체 기본 Windows 버전 (winecfg 의 "Windows 버전"과 같은 값).
/// Proton-CachyOS 는 prefix 를 Win11(빌드 22000)로 만들기 때문에 기본값으로 덮어쓴다.
pub const FIX_WIN10_PREFIX: Fix = Fix {
    id: "win10-prefix",
    key: r"Software\\Wine",
    name: "Version",
    value: "win10",
    title: "Windows 10으로 맞추기 (전체)",
    why: "이럴 때: 카카오톡 설치 창이나 오류 알림 창의 모양이 이상할 때. Proton-CachyOS가 처음에 Windows 11로 만들어서, 보통은 자동으로 고쳐집니다",
};

pub const FIX_WIN10: Fix = Fix {
    id: "win10",
    key: r"Software\\Wine\\AppDefaults\\KakaoTalkUI.exe",
    name: "Version",
    value: "win10",
    title: "카카오톡 창 테두리 고치기 (Windows 10 지정)",
    why: "이럴 때: 카카오톡 창 양옆에 하얀 줄(테두리)이 삐져나와 보일 때",
};

pub const FIX_INPUT: Fix = Fix {
    id: "input",
    key: r"Software\\Wine\\X11 Driver",
    name: "InputStyle",
    // Proton-CachyOS 11 + ibus 에서 검증: 커서 위치에서 바로 조합, 별도 미리보기 창 없음.
    // (GE-Proton 시절 조합 문제로 쓰던 "root" 는 화면 구석에 미리보기 창이 따로 뜸)
    value: "overthespot",
    title: "한글 입력 고치기",
    why: "이럴 때: 한글이 입력창이 아니라 화면 구석의 작은 창에서 조합되거나, 글자가 깨지거나 두 번씩 입력될 때",
};

pub const FIXES: [&Fix; 3] = [&FIX_WIN10_PREFIX, &FIX_WIN10, &FIX_INPUT];

fn header_key(line: &str) -> Option<&str> {
    let l = line.trim_start();
    let rest = l.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

fn value_prefix(name: &str) -> String {
    format!("\"{name}\"=")
}

/// 섹션 범위 (헤더 줄 index, 끝 index(exclusive))
fn section(lines: &[&str], key: &str) -> Option<(usize, usize)> {
    let start = lines
        .iter()
        .position(|l| header_key(l).is_some_and(|k| k.eq_ignore_ascii_case(key)))?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| header_key(l).is_some())
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Some((start, end))
}

pub fn get_value(content: &str, key: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let (s, e) = section(&lines, key)?;
    let pre = value_prefix(name).to_ascii_lowercase();
    lines[s + 1..e].iter().find_map(|l| {
        let t = l.trim_start();
        if t.to_ascii_lowercase().starts_with(&pre) {
            let v = t[pre.len()..].trim();
            Some(v.trim_matches('"').to_string())
        } else {
            None
        }
    })
}

pub fn set_value(content: &str, key: &str, name: &str, value: &str) -> String {
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let new_line = format!("\"{name}\"=\"{value}\"");
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    match section(&refs, key) {
        Some((s, e)) => {
            let pre = value_prefix(name).to_ascii_lowercase();
            if let Some(i) =
                (s + 1..e).find(|&i| lines[i].trim_start().to_ascii_lowercase().starts_with(&pre))
            {
                lines[i] = new_line;
            } else {
                // 섹션 끝의 빈 줄 앞에 삽입
                let mut at = e;
                while at > s + 1 && lines[at - 1].trim().is_empty() {
                    at -= 1;
                }
                lines.insert(at, new_line);
            }
        }
        None => {
            while lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.pop();
            }
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            lines.push(String::new());
            lines.push(format!("[{key}] {ts}"));
            lines.push(new_line);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn user_reg(prefix: &Path) -> std::path::PathBuf {
    prefix.join("user.reg")
}

/// 각 수정 항목이 이미 적용돼 있는지
pub fn check(prefix: &Path) -> Result<Vec<(&'static Fix, bool)>, String> {
    let p = user_reg(prefix);
    if !p.is_file() {
        return Err("prefix에 user.reg가 없음 — 카카오톡 설치 후 점검됩니다".into());
    }
    let c = fs::read_to_string(&p).map_err(|e| format!("user.reg 읽기 실패: {e}"))?;
    Ok(FIXES
        .iter()
        .map(|f| {
            let ok = get_value(&c, f.key, f.name).is_some_and(|v| v.eq_ignore_ascii_case(f.value));
            (*f, ok)
        })
        .collect())
}

/// 적용. 호출 전에 wineserver 가 꺼져 있는지 확인할 것.
pub fn apply(prefix: &Path, fixes: &[&Fix]) -> Result<usize, String> {
    if !crate::util::prefix_procs(prefix).is_empty() {
        return Err("카카오톡/wineserver 가 실행 중이라 레지스트리를 수정할 수 없습니다".into());
    }
    let p = user_reg(prefix);
    let orig = fs::read_to_string(&p).map_err(|e| format!("{} 읽기 실패: {e}", p.display()))?;
    let mut c = orig.clone();
    let mut changed = 0;
    for f in fixes {
        if !get_value(&c, f.key, f.name).is_some_and(|v| v.eq_ignore_ascii_case(f.value)) {
            c = set_value(&c, f.key, f.name, f.value);
            changed += 1;
        }
    }
    if changed > 0 {
        let _ = fs::write(prefix.join("user.reg.launcher-bak"), &orig);
        crate::util::write_atomic(&p, c.as_bytes())?;
    }
    Ok(changed)
}

/// prefix 의 Windows 빌드 번호 (system.reg). Proton-CachyOS 는 22000(Win11) 으로 설정함.
pub fn windows_build(prefix: &Path) -> Option<String> {
    let c = fs::read_to_string(prefix.join("system.reg")).ok()?;
    get_value(
        &c,
        r"Software\\Microsoft\\Windows NT\\CurrentVersion",
        "CurrentBuild",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "WINE REGISTRY Version 2\n;; All keys relative to \\\\User\\\\S-1-5-21\n\n#arch=win64\n\n[Software\\\\Wine\\\\X11 Driver] 1731000000\n#time=1db1234\n\"Decorated\"=\"Y\"\n\n[Software\\\\Wine\\\\Other] 1731000000\n\"A\"=\"B\"\n";

    #[test]
    fn set_and_get() {
        let c = set_value(SAMPLE, FIX_INPUT.key, "InputStyle", "root");
        assert_eq!(
            get_value(&c, FIX_INPUT.key, "InputStyle").as_deref(),
            Some("root")
        );
        assert_eq!(
            get_value(&c, FIX_INPUT.key, "Decorated").as_deref(),
            Some("Y")
        );
        assert!(c.contains("\"InputStyle\"=\"root\"\n\n[Software\\\\Wine\\\\Other]"));
        let c2 = set_value(&c, FIX_INPUT.key, "InputStyle", "root");
        assert_eq!(c, c2);
        // [Software\\Wine] 는 하위 키([Software\\Wine\\X11 Driver])와 섞이지 않아야 함
        let g = set_value(&c, FIX_WIN10_PREFIX.key, "Version", "win10");
        assert_eq!(
            get_value(&g, FIX_WIN10_PREFIX.key, "Version").as_deref(),
            Some("win10")
        );
        assert_eq!(get_value(&g, FIX_INPUT.key, "Version"), None);
        assert!(g.contains("\n[Software\\\\Wine] "));
        let c3 = set_value(&c, FIX_WIN10.key, "Version", "win10");
        assert_eq!(
            get_value(&c3, FIX_WIN10.key, "Version").as_deref(),
            Some("win10")
        );
        assert!(c3.contains("[Software\\\\Wine\\\\AppDefaults\\\\KakaoTalkUI.exe] "));
    }
}
