# Ubuntu-Kakaotalk

Ubuntu에서 **Windows용 카카오톡**을 Proton으로 실행하는 런처입니다.
설치 파일만 고르면 나머지(Proton, 실행 환경, 화면·한글 입력 설정)는 런처가 알아서 준비합니다.

> 카카오톡의 비공식 실행 도구입니다. 카카오와 관련이 없으며, 카카오톡 프로그램은 포함하지 않습니다.

## 설치 방법

**Ubuntu 24.04 이상**이 필요합니다. 터미널에서 순서대로 실행하세요.

**1. 필요한 패키지**

```bash
sudo apt install -y git curl python3 build-essential pkg-config libgtk-4-dev libadwaita-1-dev icoutils
```

```bash
sudo dpkg --add-architecture i386 && sudo apt update && sudo apt install -y libvulkan1:i386 mesa-vulkan-drivers:i386
```

**2. Rust**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . ~/.cargo/env
```

**3. 런처 설치**

```bash
git clone https://github.com/buuung/Ubuntu-Kakaotalk.git && cd Ubuntu-Kakaotalk && ./install.sh
```

**4. Ubuntu 보안 설정 허용** (이걸 안 하면 카카오톡이 뜨지 않습니다)

```bash
~/.local/bin/kakaotalk-proton --apparmor-profile | sudo tee /etc/apparmor.d/kakaotalk-launcher-umu >/dev/null && sudo apparmor_parser -r /etc/apparmor.d/kakaotalk-launcher-umu
```

**5. 카카오톡 설치**

1. [카카오 공식 사이트](https://www.kakaocorp.com/page/service/service/KakaoTalk)에서 **Windows용** 설치 파일을 받습니다.
2. 앱 메뉴에서 **KakaoTalk**을 엽니다.
3. **처음 설정**에서 **"설치 파일 선택…"**을 누르고 받은 파일을 고릅니다.
4. 설치 창이 뜨면 안내대로 진행합니다. 처음 한 번은 준비 과정 때문에 몇 분 걸립니다.

이후로는 앱 메뉴에서 **KakaoTalk**을 누르면 바로 실행됩니다.

### 앱 메뉴(애플리케이션 목록)의 KakaoTalk 바꾸기
`install.sh`가 앱 메뉴에 **KakaoTalk**을 등록합니다. 이미 Lutris나 Bottles로 만든 KakaoTalk 바로가기가 있었다면 이 런처로 바뀌고, 원래 파일은 `~/.local/share/applications/kakaotalk.desktop.bak`으로 남습니다.

앱 메뉴에 KakaoTalk이 없거나, 누르면 예전(Lutris·Bottles) 카카오톡이 열린다면 다시 등록하세요.

- 런처 창에서: **설정** 탭 → **바로가기** → **"이 앱으로 교체"**
- 또는 터미널에서:

```bash
~/.local/bin/kakaotalk-proton --install-desktop
```

바로 안 보이면 로그아웃했다가 다시 로그인하세요. 예전 바로가기로 되돌리려면 `.bak` 파일을 원래 이름으로 바꾸면 됩니다.

```bash
mv ~/.local/share/applications/kakaotalk.desktop.bak ~/.local/share/applications/kakaotalk.desktop
```

## 수리 방법

앱 메뉴의 KakaoTalk 아이콘을 **오른쪽 클릭 → "런처 설정"**을 누르고, **수리** 탭에서 증상에 맞는 버튼을 누르세요.

| 이런 증상이면 | 누를 버튼 |
|---|---|
| 카카오톡 창 **양옆에 하얀 줄**이 삐져나와 보임 | 카카오톡 창 테두리 고치기 |
| 설치 창·알림 창 등 **다른 창 모양도 이상함** | Windows 10으로 맞추기 (전체) |
| 한글이 **화면 구석 작은 창에서 조합**되거나, **깨지거나 두 번** 입력됨 | 한글 입력 고치기 |
| 이모지가 **네모(□)**로 보임 | 이모지 폰트 → 설치 / 업데이트 (설치 후 카카오톡 설정 → 화면 → 글씨체에서 'Winemoji' 글꼴 선택) |
| 앱 메뉴 아이콘이 **기본 말풍선 아이콘** | 아이콘 고치기 |
| 앱 메뉴에 KakaoTalk이 **없거나 예전 카카오톡**(Lutris·Bottles)이 열림 | 설정 탭 → 바로가기 → 이 앱으로 교체 ([자세히](#앱-메뉴애플리케이션-목록의-kakaotalk-바꾸기)) |
| 잘 모르겠음 | 원클릭 수리 |
| 카카오톡이 **멈춰서 안 꺼짐** | 실행 탭 → 강제 종료 |

그래도 안 되면 **로그** 탭을 보세요. 알려진 문제는 원인과 해결법을 위쪽에 보여줍니다.

## 라이선스

[MIT](LICENSE)
