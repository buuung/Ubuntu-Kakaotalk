#!/usr/bin/env bash
# 카카오톡 Proton 런처 빌드 & 설치
#   ./install.sh            빌드 → ~/.local/bin/kakaotalk-proton 설치 → 앱 메뉴 바로가기 교체
#   ./install.sh --no-desktop   바로가기는 건드리지 않음
set -euo pipefail
cd "$(dirname "$0")"

BIN_DIR="$HOME/.local/bin"
BIN="$BIN_DIR/kakaotalk-proton"

# 빌드 의존성: C 컴파일러, pkg-config, GTK4, libadwaita (README '설치' 참고)
need_pkgs=()
command -v cc >/dev/null || need_pkgs+=(build-essential)
if command -v pkg-config >/dev/null; then
  pkg-config --exists 'gtk4 >= 4.10' 2>/dev/null || need_pkgs+=(libgtk-4-dev)
  pkg-config --exists 'libadwaita-1 >= 1.4' 2>/dev/null || need_pkgs+=(libadwaita-1-dev)
else
  need_pkgs+=(pkg-config libgtk-4-dev libadwaita-1-dev)
fi
if ((${#need_pkgs[@]})); then
  echo "빌드에 필요한 패키지가 없거나 버전이 낮습니다: ${need_pkgs[*]}"
  echo "  sudo apt install -y ${need_pkgs[*]}"
  echo "(GTK 4.10 / libadwaita 1.4 이상 필요 → Ubuntu 24.04 이상)"
  exit 1
fi
if ! command -v cargo >/dev/null; then
  echo "Rust(cargo)가 없습니다: https://rustup.rs  (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh)"
  exit 1
fi

cargo build --release
mkdir -p "$BIN_DIR"
install -m 755 target/release/kakaotalk-proton "$BIN"
echo "설치됨: $BIN"

# 실행에 필요한 도구 확인
warn=()
command -v python3 >/dev/null || warn+=(python3)
command -v curl >/dev/null || command -v wget >/dev/null || warn+=(curl)
command -v fc-cache >/dev/null || warn+=(fontconfig)
command -v notify-send >/dev/null || warn+=(libnotify-bin)
command -v xdg-open >/dev/null || warn+=(xdg-utils)
if ((${#warn[@]})); then
  echo "실행에 필요한 도구 누락: ${warn[*]}"
  echo "  sudo apt install -y ${warn[*]}"
fi
command -v wrestool >/dev/null || echo "(선택) 앱 아이콘 추출: sudo apt install icoutils"
command -v vulkaninfo >/dev/null || echo "(선택) GPU 자동 선택 정확도: sudo apt install vulkan-tools"

# 32비트 라이브러리 확인 (Lutris가 누락 경고했던 것들)
missing=()
for p in libvulkan1:i386 mesa-vulkan-drivers:i386 libgl1:i386 libgnutls30:i386; do
  dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
done
if ((${#missing[@]})); then
  echo
  echo "⚠ 32비트 패키지 누락: ${missing[*]}"
  echo "  이게 없으면 카카오톡이 '준비 중'에서 멈출 수 있습니다 (Lutris 무한로딩과 같은 원인)."
  cmd="sudo dpkg --add-architecture i386 && sudo apt update && sudo apt install -y ${missing[*]}"
  echo "  $cmd"
  if [[ -t 0 ]]; then
    read -r -p "  지금 설치할까요? [Y/n] " ans
    if [[ -z "$ans" || "$ans" =~ ^[Yy] ]]; then
      bash -c "$cmd" || echo "  설치에 실패했습니다. 위 명령을 직접 실행하세요."
    fi
  fi
fi

if [[ "${1:-}" != "--no-desktop" ]]; then
  "$BIN" --install-desktop
  echo "앱 메뉴의 KakaoTalk 이 이제 이 런처로 실행됩니다 (기존 파일은 kakaotalk.desktop.bak)."
fi

"$BIN" --status || true
