//! GTK4 + libadwaita 설정 창

use crate::config::{self, Config};
use crate::runner::{self, Level};
use crate::util::{self, RunState};
use crate::{desktop, fonts, gpu, proton, registry, sysdeps, umu};
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const APP_ID: &str = "io.github.buuung.KakaoTalkLauncher";
/// 카카오 공식 서비스 페이지 (PC 버전 다운로드 링크가 있는 곳). 설치 파일은 사용자가 직접 받는다.
const KAKAO_PAGE: &str = "https://www.kakaocorp.com/page/service/service/KakaoTalk";

/// `page`: 처음 보여줄 탭 (home / settings / repair / logs)
pub fn run(auto_launch: bool, page: Option<String>) -> ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        let ui = Ui::build(app);
        if let Some(p) = &page {
            ui.stack.set_visible_child_name(p);
        }
        ui.win.present();
        if auto_launch {
            ui.launch();
        }
    });
    let code = app.run_with_args::<&str>(&[]);
    if code == glib::ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// 무거운 작업은 스레드에서, 결과는 메인 루프에서
fn bg<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(v) => {
            if let Some(d) = done.take() {
                d(v);
            }
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

fn row(title: &str, subtitle: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    r.set_subtitle_selectable(true);
    r
}

fn flat_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build()
}

fn status_icon(ok: bool) -> gtk::Image {
    let i = gtk::Image::from_icon_name(if ok {
        "emblem-ok-symbolic"
    } else {
        "dialog-warning-symbolic"
    });
    i.add_css_class(if ok { "success" } else { "warning" });
    i
}

fn set_status(icon: &gtk::Image, ok: bool) {
    icon.set_icon_name(Some(if ok {
        "emblem-ok-symbolic"
    } else {
        "dialog-warning-symbolic"
    }));
    icon.remove_css_class(if ok { "warning" } else { "success" });
    icon.add_css_class(if ok { "success" } else { "warning" });
}

/// 처음 설정 단계 한 줄: (행, 상태 아이콘, 동작 버튼)
fn step_row(title: &str, button: &str) -> (adw::ActionRow, gtk::Image, gtk::Button) {
    let r = row(title, "");
    let i = status_icon(false);
    r.add_prefix(&i);
    let b = flat_button(button);
    r.add_suffix(&b);
    (r, i, b)
}

/// 설치 전에 Proton 등을 먼저 받아야 할 때, 끝나면 이어서 할 일
enum Pending {
    Install(std::path::PathBuf),
    Launch,
}

fn page_scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    page_scroller_max(child, 720)
}

fn page_scroller_max(child: &impl IsA<gtk::Widget>, max: i32) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder()
        .maximum_size(max)
        .tightening_threshold(max * 3 / 4)
        .unit(adw::LengthUnit::Px)
        .child(child)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build()
}

fn vbox(spacing: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, spacing);
    b.set_margin_top(18);
    b.set_margin_bottom(18);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b
}

struct Ui {
    cfg: RefCell<Config>,
    win: adw::ApplicationWindow,
    stack: adw::ViewStack,
    toasts: adw::ToastOverlay,

    // 실행 탭
    status_title: gtk::Label,
    status_desc: gtk::Label,
    launch_btn: gtk::Button,
    kill_btn: gtk::Button,
    progress: gtk::ProgressBar,
    progress_label: gtk::Label,
    home_banner: adw::Banner,
    info_umu: adw::ActionRow,
    info_proton: adw::ActionRow,
    info_rt: adw::ActionRow,
    info_gpu: adw::ActionRow,
    starting_since: Cell<Option<Instant>>,
    info_kakao: adw::ActionRow,
    installing: Cell<bool>,
    killing: Cell<bool>,
    icon_row: adw::ActionRow,
    icon_state: gtk::Image,
    /// 다운로드 등 오래 걸리는 작업 중 실행 탭에 보여줄 문구
    busy: RefCell<Option<String>>,
    proton_dl_btn: gtk::Button,
    proton_dl_row: adw::ActionRow,
    proton_dl_bar: gtk::ProgressBar,
    proton_dl_active: Cell<bool>,
    pending: RefCell<Option<Pending>>,
    umu_busy: Cell<bool>,
    setup_group: adw::PreferencesGroup,
    step_userns: (adw::ActionRow, gtk::Image, gtk::Button),
    step_vk: (adw::ActionRow, gtk::Image, gtk::Button),
    step_umu: (adw::ActionRow, gtk::Image, gtk::Button),
    step_proton: (adw::ActionRow, gtk::Image, gtk::Button),
    step_kakao: (adw::ActionRow, gtk::Image, gtk::Button),
    step_font: (adw::ActionRow, gtk::Image, gtk::Button),
    last_state: Cell<Option<RunState>>,

    // 설정 탭
    proton_row: adw::ComboRow,
    proton_names: RefCell<Vec<String>>,
    gpu_row: adw::ComboRow,
    gpu_values: RefCell<Vec<String>>,
    gpu_info: adw::ActionRow,
    gpus: Vec<gpu::Gpu>,
    vk_devices: RefCell<Vec<String>>,
    umu_row: adw::ActionRow,
    env_buffer: gtk::TextBuffer,
    env_error: gtk::Label,
    desktop_row: adw::ActionRow,

    // 수리 탭
    fix_rows: RefCell<Vec<(&'static registry::Fix, adw::ActionRow, gtk::Image)>>,
    winbuild_row: adw::ActionRow,
    font_row: adw::ComboRow,
    font_names: RefCell<Vec<String>>,
    font_status: adw::ActionRow,

    // 로그 탭
    log_buffer: gtk::TextBuffer,
    log_view: gtk::TextView,
    log_banner: adw::Banner,
    findings_box: gtk::ListBox,
    show_prev: gtk::ToggleButton,
    tail_offset: Cell<u64>,
    last_line: RefCell<String>,
}

impl Ui {
    fn build(app: &adw::Application) -> Rc<Self> {
        let cfg = Config::load();

        // ───────── 실행 탭 ─────────
        let launch_btn = gtk::Button::builder().label("실행").build();
        launch_btn.add_css_class("pill");
        launch_btn.add_css_class("suggested-action");
        let kill_btn = gtk::Button::builder().label("강제 종료").build();
        kill_btn.add_css_class("pill");
        kill_btn.add_css_class("destructive-action");
        let btns = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        btns.set_halign(gtk::Align::Center);
        btns.append(&launch_btn);
        btns.append(&kill_btn);

        let progress = gtk::ProgressBar::new();
        progress.set_visible(false);
        let progress_label = gtk::Label::builder()
            .wrap(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .lines(2)
            .visible(false)
            .build();
        progress_label.add_css_class("dim-label");
        progress_label.add_css_class("caption");

        // ── 처음 설정 (하나라도 빠지면 표시) ──
        let step_userns = step_row("Ubuntu 보안 설정 (AppArmor)", "설치 명령 복사");
        let step_vk = step_row("① 32비트 Vulkan 드라이버", "설치 명령 복사");
        let step_umu = step_row("② umu-launcher", "받기");
        let step_proton = step_row("③ Proton-CachyOS", "받기");
        let step_kakao = step_row("④ 카카오톡 설치", "설치 파일 선택…");
        step_kakao.2.add_css_class("suggested-action");
        let step_page_btn = gtk::Button::from_icon_name("web-browser-symbolic");
        step_page_btn.set_valign(gtk::Align::Center);
        step_page_btn.add_css_class("flat");
        step_page_btn.set_tooltip_text(Some("카카오 공식 다운로드 페이지 열기"));
        step_kakao.0.add_suffix(&step_page_btn);
        let step_font = step_row("이모지 폰트 (선택)", "설치");
        let setup_group = adw::PreferencesGroup::builder()
            .title("처음 설정")
            .description("설치 파일만 고르면 umu-launcher와 Proton은 필요할 때 자동으로 받습니다")
            .build();
        for r in [
            &step_userns.0,
            &step_vk.0,
            &step_umu.0,
            &step_proton.0,
            &step_kakao.0,
            &step_font.0,
        ] {
            setup_group.add(r);
        }

        let info_kakao = row("카카오톡", "");
        let pick_btn = gtk::Button::from_icon_name("document-open-symbolic");
        pick_btn.set_valign(gtk::Align::Center);
        pick_btn.add_css_class("flat");
        pick_btn.set_tooltip_text(Some("설치 파일 선택 (KakaoTalk_Setup.exe)"));
        let page_btn = gtk::Button::from_icon_name("web-browser-symbolic");
        page_btn.set_valign(gtk::Align::Center);
        page_btn.add_css_class("flat");
        page_btn.set_tooltip_text(Some("카카오 공식 다운로드 페이지 열기"));
        info_kakao.add_suffix(&page_btn);
        info_kakao.add_suffix(&pick_btn);
        let info_umu = row("umu-run", "");
        let info_proton = row("Proton", "");
        let info_rt = row("Steam Runtime", "");
        let info_gpu = row("GPU (DXVK)", "");
        let info_group = adw::PreferencesGroup::builder().title("구성").build();
        info_group.add(&info_kakao);
        info_group.add(&info_umu);
        info_group.add(&info_proton);
        info_group.add(&info_rt);
        info_group.add(&info_gpu);

        // AdwStatusPage 는 내용을 자연 폭으로 가운데 정렬해서 좁아지므로 직접 배치
        let has_icon = gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap())
            .has_icon(crate::icon::NAME);
        let hero = gtk::Image::from_icon_name(if has_icon {
            crate::icon::NAME
        } else {
            "user-available-symbolic"
        });
        hero.set_pixel_size(96);
        if !has_icon {
            hero.add_css_class("dim-label");
        }
        let status_title = gtk::Label::builder().label("카카오톡").wrap(true).build();
        status_title.add_css_class("title-1");
        let status_desc = gtk::Label::builder()
            .label("확인 중…")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        status_desc.add_css_class("dim-label");
        let head = gtk::Box::new(gtk::Orientation::Vertical, 6);
        head.append(&hero);
        head.append(&status_title);
        head.append(&status_desc);
        head.set_margin_bottom(6);

        let home_inner = gtk::Box::new(gtk::Orientation::Vertical, 18);
        home_inner.set_margin_top(36);
        home_inner.set_margin_bottom(24);
        home_inner.set_margin_start(12);
        home_inner.set_margin_end(12);
        home_inner.append(&head);
        home_inner.append(&btns);
        home_inner.append(&progress);
        home_inner.append(&progress_label);
        home_inner.append(&setup_group);
        home_inner.append(&info_group);
        let home_scroll = page_scroller_max(&home_inner, 520);
        let home_banner = adw::Banner::builder().button_label("로그 보기").build();
        let home = gtk::Box::new(gtk::Orientation::Vertical, 0);
        home.append(&home_banner);
        home.append(&home_scroll);

        // ───────── 설정 탭 ─────────
        let proton_row = adw::ComboRow::builder().title("Proton 버전").build();
        let proton_refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        proton_refresh.set_valign(gtk::Align::Center);
        proton_refresh.add_css_class("flat");
        proton_refresh.set_tooltip_text(Some("compatibilitytools.d 다시 검색"));
        proton_row.add_suffix(&proton_refresh);
        let proton_dl_btn = flat_button("최신 Proton-CachyOS 받기");
        proton_dl_btn.add_css_class("flat");
        let proton_group = adw::PreferencesGroup::builder()
            .title("Proton")
            .description("~/.local/share/Steam/compatibilitytools.d")
            .header_suffix(&proton_dl_btn)
            .build();
        proton_group.add(&proton_row);
        let proton_dl_bar = gtk::ProgressBar::builder()
            .valign(gtk::Align::Center)
            .width_request(140)
            .build();
        let proton_dl_row = row("Proton-CachyOS 받는 중", "");
        proton_dl_row.add_suffix(&proton_dl_bar);
        proton_dl_row.set_visible(false);
        proton_group.add(&proton_dl_row);

        let prefix_row = adw::EntryRow::builder()
            .title("WINEPREFIX")
            .text(&cfg.wineprefix)
            .build();
        let exe_row = adw::EntryRow::builder()
            .title("실행 파일 (상대 경로면 prefix 기준)")
            .text(&cfg.exe)
            .build();
        let gameid_row = adw::EntryRow::builder()
            .title("GAMEID")
            .text(&cfg.game_id)
            .build();
        let umu_entry = adw::EntryRow::builder()
            .title("umu-run 경로 (비우면 자동)")
            .text(&cfg.umu_run)
            .build();
        let gpu_row = adw::ComboRow::builder().title("DXVK가 쓸 GPU").build();
        let gpu_info = row("감지된 GPU", "확인 중…");
        let gpu_group = adw::PreferencesGroup::builder()
            .title("GPU")
            .description("DXVK_FILTER_DEVICE_NAME. 맞는 GPU가 없으면 실행이 실패하므로, 그런 경우 런처가 자동으로 필터를 끕니다")
            .build();
        gpu_group.add(&gpu_row);
        gpu_group.add(&gpu_info);

        let path_group = adw::PreferencesGroup::builder().title("경로").build();
        path_group.add(&prefix_row);
        path_group.add(&exe_row);
        path_group.add(&gameid_row);
        path_group.add(&umu_entry);

        let umu_row = row("현재 사용 중인 umu-run", "");
        let umu_copy = flat_button("Lutris에서 복사");
        let umu_dl = flat_button("GitHub에서 받기");
        umu_copy.add_css_class("flat");
        umu_dl.add_css_class("flat");
        let umu_btns = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        umu_btns.append(&umu_copy);
        umu_btns.append(&umu_dl);
        let umu_group = adw::PreferencesGroup::builder()
            .title("umu-launcher")
            .header_suffix(&umu_btns)
            .description("Lutris를 지워도 동작하도록 ~/.local/share/kakaotalk-launcher/umu/ 에 자체 사본을 둡니다")
            .build();
        umu_group.add(&umu_row);

        let env_buffer = gtk::TextBuffer::new(None);
        env_buffer.set_text(&cfg.env_text());
        let env_view = gtk::TextView::builder()
            .buffer(&env_buffer)
            .monospace(true)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .height_request(120)
            .build();
        let env_frame = gtk::Frame::builder().child(&env_view).build();
        env_frame.add_css_class("view");
        let env_error = gtk::Label::builder().xalign(0.0).visible(false).build();
        env_error.add_css_class("error");
        env_error.add_css_class("caption");
        let env_reset = flat_button("기본값");
        env_reset.add_css_class("flat");
        let env_group = adw::PreferencesGroup::builder()
            .title("추가 환경 변수")
            .description("한 줄에 KEY=VALUE. WINEPREFIX/GAMEID/PROTONPATH는 위 항목이 우선합니다")
            .header_suffix(&env_reset)
            .build();
        let env_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        env_box.append(&env_frame);
        env_box.append(&env_error);
        env_group.add(&env_box);

        let debug_row = adw::SwitchRow::builder()
            .title("디버그 로그")
            .subtitle("PROTON_LOG=1, UMU_LOG=1 (평소엔 WINEDEBUG=-all)")
            .active(cfg.debug_log)
            .build();
        let auto_row = adw::SwitchRow::builder()
            .title("실행 전 레지스트리 자동 점검·수리")
            .subtitle(
                "켜 두면 창 테두리·한글 입력 설정이 풀려도 카카오톡을 켤 때마다 알아서 다시 고칩니다 (Proton 업데이트 후에도)",
            )
            .active(cfg.auto_repair)
            .build();
        let opt_group = adw::PreferencesGroup::builder().title("옵션").build();
        opt_group.add(&debug_row);
        opt_group.add(&auto_row);

        let desktop_row = row("앱 메뉴 바로가기", "");
        let desktop_btn = flat_button("이 앱으로 교체");
        desktop_row.add_suffix(&desktop_btn);
        let desktop_group = adw::PreferencesGroup::builder()
            .title("바로가기")
            .description("기존 파일은 kakaotalk.desktop.bak 로 백업합니다. 아이콘 우클릭 메뉴에 '런처 설정'·'강제 종료'가 추가됩니다")
            .build();
        desktop_group.add(&desktop_row);

        let settings_box = vbox(24);
        for g in [
            &proton_group,
            &gpu_group,
            &path_group,
            &umu_group,
            &env_group,
            &opt_group,
            &desktop_group,
        ] {
            settings_box.append(g);
        }

        // ───────── 수리 탭 ─────────
        let repair_all = gtk::Button::builder().label("원클릭 수리").build();
        repair_all.add_css_class("suggested-action");
        repair_all.set_valign(gtk::Align::Center);
        let recheck = gtk::Button::from_icon_name("view-refresh-symbolic");
        recheck.add_css_class("flat");
        recheck.set_valign(gtk::Align::Center);
        let hdr = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        hdr.append(&recheck);
        hdr.append(&repair_all);
        let reg_group = adw::PreferencesGroup::builder()
            .title("화면·입력 문제 고치기")
            .description("카카오톡이 이상하게 보이거나 한글 입력이 안 될 때, 증상에 맞는 '적용'을 누르거나 '원클릭 수리'로 한 번에 고치세요. 카카오톡이 켜져 있으면 먼저 끄고 진행합니다")
            .header_suffix(&hdr)
            .build();
        let mut fix_rows = Vec::new();
        for f in registry::FIXES {
            let r = row(f.title, f.why);
            let icon = status_icon(false);
            r.add_prefix(&icon);
            let b = flat_button("적용");
            r.add_suffix(&b);
            reg_group.add(&r);
            fix_rows.push((f, r, icon, b));
        }
        let winbuild_row = row("현재 Windows 버전 (참고용)", "");
        reg_group.add(&winbuild_row);

        let font_row = adw::ComboRow::builder().title("Winemoji 폰트").build();
        let font_list_btn = flat_button("목록 불러오기");
        font_row.add_suffix(&font_list_btn);
        let font_status = row("이모지 폰트", "");
        let font_install = flat_button("설치 / 업데이트");
        font_status.add_suffix(&font_install);
        let font_group = adw::PreferencesGroup::builder()
            .title("이모지가 네모(□)로 보일 때")
            .description("이모지 글꼴(Winemoji)을 설치·업데이트합니다. 설치한 뒤 카카오톡 설정 → 화면 → 글씨체에서 'Winemoji'가 붙은 글꼴을 고르세요")
            .build();
        font_group.add(&font_row);
        font_group.add(&font_status);

        let icon_row = row(
            "카카오톡 아이콘",
            "이럴 때: 앱 메뉴에서 카카오톡이 말풍선 모양 기본 아이콘으로 보일 때",
        );
        let icon_state = status_icon(false);
        icon_row.add_prefix(&icon_state);
        let icon_fix = flat_button("아이콘 고치기");
        icon_row.add_suffix(&icon_fix);
        let icon_group = adw::PreferencesGroup::builder()
            .title("앱 메뉴 아이콘이 카카오톡이 아닐 때")
            .description(
                "설치된 카카오톡에서 아이콘을 꺼내 앱 메뉴에 다시 적용합니다 (icoutils 필요)",
            )
            .build();
        icon_group.add(&icon_row);

        let repair_box = vbox(24);
        repair_box.append(&reg_group);
        repair_box.append(&icon_group);
        repair_box.append(&font_group);

        let left = desktop::leftovers();
        if !left.is_empty() {
            let g = adw::PreferencesGroup::builder()
                .title("이전 설치 흔적")
                .description("안정화된 뒤 정리할지 직접 결정하세요. 이 앱은 삭제하지 않습니다. (같은 계정은 PC 한 곳만 로그인되므로 동시에 켜지 마세요)")
                .build();
            for p in left {
                let r = row(
                    &p.file_name().unwrap_or_default().to_string_lossy(),
                    &p.to_string_lossy(),
                );
                let b = flat_button("폴더 열기");
                let target = if p.is_dir() {
                    p.clone()
                } else {
                    p.parent().unwrap().to_path_buf()
                };
                b.connect_clicked(move |_| util::open_path(&target));
                r.add_suffix(&b);
                g.add(&r);
            }
            repair_box.append(&g);
        }

        // ───────── 로그 탭 ─────────
        let log_banner = adw::Banner::new("");
        let findings_box = gtk::ListBox::new();
        findings_box.add_css_class("boxed-list");
        findings_box.set_selection_mode(gtk::SelectionMode::None);
        findings_box.set_visible(false);

        let show_prev = gtk::ToggleButton::with_label("이전 실행");
        let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
        reload.set_tooltip_text(Some("다시 읽기"));
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some("복사 (비밀 값 마스킹됨)"));
        let open_dir = gtk::Button::from_icon_name("folder-open-symbolic");
        open_dir.set_tooltip_text(Some("로그 폴더 열기"));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let path_label = gtk::Label::builder()
            .label(runner::log_path().to_string_lossy().as_ref())
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .build();
        path_label.add_css_class("dim-label");
        path_label.add_css_class("caption");
        bar.append(&path_label);
        bar.append(&show_prev);
        bar.append(&reload);
        bar.append(&copy);
        bar.append(&open_dir);

        let log_buffer = gtk::TextBuffer::new(None);
        let log_view = gtk::TextView::builder()
            .buffer(&log_buffer)
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .build();
        let log_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&log_view)
            .build();
        let log_frame = gtk::Frame::builder()
            .child(&log_scroll)
            .vexpand(true)
            .build();

        let logs_inner = gtk::Box::new(gtk::Orientation::Vertical, 12);
        logs_inner.set_margin_top(12);
        logs_inner.set_margin_bottom(12);
        logs_inner.set_margin_start(12);
        logs_inner.set_margin_end(12);
        logs_inner.append(&findings_box);
        logs_inner.append(&bar);
        logs_inner.append(&log_frame);
        let logs = gtk::Box::new(gtk::Orientation::Vertical, 0);
        logs.append(&log_banner);
        logs.append(&logs_inner);

        // ───────── 창 ─────────
        let stack = adw::ViewStack::new();
        stack.add_titled_with_icon(&home, Some("home"), "실행", "media-playback-start-symbolic");
        stack.add_titled_with_icon(
            &page_scroller(&settings_box),
            Some("settings"),
            "설정",
            "emblem-system-symbolic",
        );
        stack.add_titled_with_icon(
            &page_scroller(&repair_box),
            Some("repair"),
            "수리",
            "applications-engineering-symbolic",
        );
        stack.add_titled_with_icon(&logs, Some("logs"), "로그", "utilities-terminal-symbolic");

        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.add_bottom_bar(&switcher_bar);
        tv.set_content(Some(&toasts));

        let win = adw::ApplicationWindow::builder()
            .application(app)
            .title("카카오톡 런처")
            .default_width(760)
            .default_height(680)
            .width_request(360)
            .height_request(400)
            .content(&tv)
            .build();
        // 좁은 창에서는 아래 탭바로
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            560.0,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
        bp.add_setter(&switcher, "visible", Some(&false.to_value()));
        win.add_breakpoint(bp);

        let ui = Rc::new(Ui {
            cfg: RefCell::new(cfg),
            win,
            stack,
            toasts,
            status_title,
            status_desc,
            launch_btn: launch_btn.clone(),
            kill_btn: kill_btn.clone(),
            progress,
            progress_label,
            home_banner: home_banner.clone(),
            info_umu,
            info_proton,
            info_rt,
            info_gpu,
            starting_since: Cell::new(None),
            info_kakao,
            installing: Cell::new(false),
            killing: Cell::new(false),
            icon_row: icon_row.clone(),
            icon_state: icon_state.clone(),
            busy: RefCell::new(None),
            proton_dl_btn: proton_dl_btn.clone(),
            proton_dl_row,
            proton_dl_bar,
            proton_dl_active: Cell::new(false),
            pending: RefCell::new(None),
            umu_busy: Cell::new(false),
            setup_group,
            step_userns: step_userns.clone(),
            step_vk: step_vk.clone(),
            step_umu: step_umu.clone(),
            step_proton: step_proton.clone(),
            step_kakao: step_kakao.clone(),
            step_font: step_font.clone(),
            last_state: Cell::new(None),
            proton_row: proton_row.clone(),
            proton_names: RefCell::new(Vec::new()),
            gpu_row: gpu_row.clone(),
            gpu_values: RefCell::new(Vec::new()),
            gpu_info,
            gpus: gpu::detect(),
            vk_devices: RefCell::new(Vec::new()),
            umu_row,
            env_buffer: env_buffer.clone(),
            env_error,
            desktop_row,
            fix_rows: RefCell::new(
                fix_rows
                    .iter()
                    .map(|(f, r, i, _)| (*f, r.clone(), i.clone()))
                    .collect(),
            ),
            winbuild_row,
            font_row: font_row.clone(),
            font_names: RefCell::new(Vec::new()),
            font_status,
            log_buffer,
            log_view,
            log_banner: log_banner.clone(),
            findings_box,
            show_prev: show_prev.clone(),
            tail_offset: Cell::new(0),
            last_line: RefCell::new(String::new()),
        });

        // ───────── 시그널 ─────────
        let u = ui.clone();
        launch_btn.connect_clicked(move |_| {
            if u.cfg.borrow().exe_path().is_file() {
                u.launch();
            } else {
                u.pick_installer();
            }
        });
        let u = ui.clone();
        step_userns.2.connect_clicked(move |b| {
            b.clipboard().set_text(&sysdeps::apparmor_install_cmd());
            u.toast("명령을 복사했습니다. 터미널에 붙여넣어 실행하세요 (관리자 암호 필요)");
        });
        step_vk.2.connect_clicked(|b| {
            b.clipboard().set_text(sysdeps::VULKAN32_CMD);
        });
        let u = ui.clone();
        step_vk.2.connect_clicked(move |_| {
            u.toast("명령을 복사했습니다. 터미널에 붙여넣어 실행하세요");
        });
        let u = ui.clone();
        step_umu.2.connect_clicked(move |_| u.with_umu(|| {}));
        let u = ui.clone();
        step_proton.2.connect_clicked(move |_| u.download_proton());
        let u = ui.clone();
        step_kakao.2.connect_clicked(move |_| u.pick_installer());
        step_page_btn.connect_clicked(|_| util::open_uri(KAKAO_PAGE));
        let u = ui.clone();
        step_font.2.connect_clicked(move |b| {
            b.set_sensitive(false);
            let name = u.cfg.borrow().font_file.clone();
            let (u2, b2) = (u.clone(), b.clone());
            bg(
                move || fonts::install(&name),
                move |r| {
                    b2.set_sensitive(true);
                    match r {
                        Ok(m) | Err(m) => u2.toast(&m),
                    }
                    u2.refresh_checks();
                },
            );
        });
        let u = ui.clone();
        pick_btn.connect_clicked(move |_| u.pick_installer());
        page_btn.connect_clicked(|_| util::open_uri(KAKAO_PAGE));
        let u = ui.clone();
        proton_dl_btn.connect_clicked(move |_| u.download_proton());
        let u = ui.clone();
        kill_btn.connect_clicked(move |_| u.kill(None));
        let u = ui.clone();
        icon_fix.connect_clicked(move |b| {
            b.set_sensitive(false);
            let exe = u.cfg.borrow().exe_path();
            let (u2, b2) = (u.clone(), b.clone());
            bg(
                move || -> Result<String, String> {
                    let p = crate::icon::regenerate(&exe)?;
                    // 바로가기가 없거나 이 런처를 가리키면 Icon= 갱신
                    let me = std::env::current_exe().map_err(|e| e.to_string())?;
                    let note = match desktop::current_exec() {
                        Some(_) if !desktop::points_to(&me) => {
                            " — 단, 앱 메뉴 바로가기가 이 런처를 가리키지 않아 그대로 둡니다 (설정 탭 '이 앱으로 교체')"
                        }
                        _ => {
                            desktop::install(&me)?;
                            ""
                        }
                    };
                    Ok(format!("아이콘을 고쳤습니다: {}{note}", p.display()))
                },
                move |r| {
                    b2.set_sensitive(true);
                    match r {
                        Ok(m) => u2.toast(&m),
                        Err(e) => u2.show_error("아이콘을 고치지 못했습니다", &e, None),
                    }
                    u2.refresh_checks();
                    u2.refresh_info();
                },
            );
        });
        for b in [&home_banner, &log_banner] {
            let u = ui.clone();
            b.connect_button_clicked(move |_| u.stack.set_visible_child_name("logs"));
        }

        ui.fill_proton_list();
        ui.fill_gpu_list();
        let u = ui.clone();
        gpu_row.connect_selected_notify(move |r| {
            let v = u.gpu_values.borrow().get(r.selected() as usize).cloned();
            if let Some(v) = v
                && u.cfg.borrow().gpu != v
            {
                u.edit(|c| c.gpu = v);
            }
            u.refresh_gpu();
        });
        // vulkaninfo 는 느릴 수 있어 백그라운드에서
        let u = ui.clone();
        bg(gpu::vulkan_devices, move |vk| {
            *u.vk_devices.borrow_mut() = vk;
            u.refresh_gpu();
        });
        let u = ui.clone();
        proton_row.connect_selected_notify(move |r| {
            let names = u.proton_names.borrow();
            if let Some(n) = names.get(r.selected() as usize)
                && u.cfg.borrow().proton != *n
            {
                u.edit(|c| c.proton = n.clone());
            }
            drop(names);
            u.refresh_info();
        });
        let u = ui.clone();
        proton_refresh.connect_clicked(move |_| u.fill_proton_list());

        let u = ui.clone();
        prefix_row.connect_changed(move |e| {
            let t = e.text().to_string();
            u.edit(|c| c.wineprefix = t);
            u.refresh_checks();
        });
        let u = ui.clone();
        exe_row.connect_changed(move |e| {
            let t = e.text().to_string();
            u.edit(|c| c.exe = t);
        });
        let u = ui.clone();
        gameid_row.connect_changed(move |e| {
            let t = e.text().to_string();
            u.edit(|c| c.game_id = t);
        });
        let u = ui.clone();
        umu_entry.connect_changed(move |e| {
            let t = e.text().to_string();
            u.edit(|c| c.umu_run = t);
            u.refresh_info();
        });
        let u = ui.clone();
        umu_copy.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (u2, b2) = (u.clone(), b.clone());
            bg(umu::copy_from_lutris, move |r| {
                b2.set_sensitive(true);
                match r {
                    Ok(p) => u2.toast(&format!("복사 완료: {}", p.display())),
                    Err(e) => u2.toast(&e),
                }
                u2.refresh_info();
            });
        });
        let u = ui.clone();
        umu_dl.connect_clicked(move |b| {
            b.set_sensitive(false);
            u.toast("umu-launcher 최신 릴리스 받는 중…");
            let (u2, b2) = (u.clone(), b.clone());
            bg(umu::download_latest, move |r| {
                b2.set_sensitive(true);
                match r {
                    Ok(tag) => u2.toast(&format!("umu-launcher {tag} 설치 완료")),
                    Err(e) => u2.toast(&format!("받기 실패: {e}")),
                }
                u2.refresh_info();
            });
        });

        let u = ui.clone();
        env_buffer.connect_changed(move |b| {
            let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
            let (map, bad) = config::parse_env_text(&text);
            if bad.is_empty() {
                u.env_error.set_visible(false);
            } else {
                u.env_error.set_label(&format!(
                    "{}번째 줄은 KEY=VALUE 형식이 아니라 무시됩니다",
                    bad.iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                u.env_error.set_visible(true);
            }
            let warn = map.get("PROTON_USE_WINED3D").is_some_and(|v| v == "1");
            if warn {
                u.env_error
                    .set_label("⚠ PROTON_USE_WINED3D=1 은 카카오톡(Qt6) 크래시 원인입니다");
                u.env_error.set_visible(true);
            }
            u.edit(|c| c.env = map);
            u.refresh_gpu();
        });
        let u = ui.clone();
        env_reset.connect_clicked(move |_| {
            u.env_buffer.set_text(
                &Config {
                    env: config::default_env(),
                    ..Config::default()
                }
                .env_text(),
            );
        });
        let u = ui.clone();
        debug_row.connect_active_notify(move |r| {
            let a = r.is_active();
            u.edit(|c| c.debug_log = a);
        });
        let u = ui.clone();
        auto_row.connect_active_notify(move |r| {
            let a = r.is_active();
            u.edit(|c| c.auto_repair = a);
        });
        let u = ui.clone();
        desktop_btn.connect_clicked(move |_| {
            match std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|e| desktop::install(&e))
            {
                Ok(_) => u.toast("앱 메뉴 바로가기를 교체했습니다"),
                Err(e) => u.toast(&e),
            }
            u.refresh_info();
        });

        for (f, _, _, b) in &fix_rows {
            let (u, f) = (ui.clone(), *f);
            b.connect_clicked(move |_| u.repair(vec![f], false));
        }
        let u = ui.clone();
        repair_all.connect_clicked(move |_| u.repair(registry::FIXES.to_vec(), true));
        let u = ui.clone();
        recheck.connect_clicked(move |_| u.refresh_checks());

        ui.set_font_names(vec![ui.cfg.borrow().font_file.clone()]);
        let u = ui.clone();
        font_row.connect_selected_notify(move |r| {
            let n = u.font_names.borrow().get(r.selected() as usize).cloned();
            if let Some(n) = n
                && u.cfg.borrow().font_file != n
            {
                u.edit(|c| c.font_file = n);
            }
            u.refresh_checks();
        });
        let u = ui.clone();
        font_list_btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (u2, b2) = (u.clone(), b.clone());
            bg(fonts::list_remote, move |r| {
                b2.set_sensitive(true);
                match r {
                    Ok(list) => {
                        let n = list.len();
                        u2.set_font_names(list.into_iter().map(|f| f.name).collect());
                        u2.toast(&format!("폰트 {n}종을 찾았습니다"));
                    }
                    Err(e) => u2.toast(&e),
                }
            });
        });
        let u = ui.clone();
        font_install.connect_clicked(move |_| {
            let u2 = u.clone();
            u.ensure_stopped("폰트 설치", move || {
                let name = u2.cfg.borrow().font_file.clone();
                u2.toast("폰트 받는 중…");
                let u3 = u2.clone();
                bg(
                    move || fonts::install(&name),
                    move |r| {
                        match r {
                            Ok(m) | Err(m) => u3.toast(&m),
                        }
                        u3.refresh_checks();
                    },
                );
            });
        });

        let u = ui.clone();
        show_prev.connect_toggled(move |_| u.reload_log());
        let u = ui.clone();
        reload.connect_clicked(move |_| u.reload_log());
        let u = ui.clone();
        copy.connect_clicked(move |_| {
            let b = &u.log_buffer;
            let t = b.text(&b.start_iter(), &b.end_iter(), false);
            u.win.clipboard().set_text(&util::mask_secrets(&t));
            u.toast("로그를 복사했습니다");
        });
        open_dir.connect_clicked(move |_| {
            let d = util::state_dir();
            let _ = fs::create_dir_all(&d);
            util::open_path(&d);
        });

        // 주기 작업: 상태 1초, 로그 tail 0.3초
        let u = ui.clone();
        glib::timeout_add_local(Duration::from_millis(1000), move || {
            u.refresh_state();
            glib::ControlFlow::Continue
        });
        let u = ui.clone();
        glib::timeout_add_local(Duration::from_millis(300), move || {
            u.tail_log();
            if u.progress.is_visible() {
                u.progress.pulse();
            }
            glib::ControlFlow::Continue
        });

        ui.reload_log();
        ui.refresh_info();
        ui.refresh_checks();
        ui.refresh_state();
        ui
    }

    fn toast(&self, msg: &str) {
        let t = adw::Toast::new(msg);
        t.set_timeout(4);
        self.toasts.add_toast(t);
    }

    fn edit(&self, f: impl FnOnce(&mut Config)) {
        let mut c = self.cfg.borrow_mut();
        f(&mut c);
        if let Err(e) = c.save() {
            drop(c);
            self.toast(&format!("설정 저장 실패: {e}"));
        }
    }

    // ───────── 실행 / 종료 ─────────

    fn launch(self: &Rc<Self>) {
        let prefix = self.cfg.borrow().prefix_path();
        if util::run_state(&prefix) != RunState::Stopped {
            self.toast("이미 실행 중입니다");
            return;
        }
        let cfg = self.cfg.borrow().clone();
        if !cfg.exe_path().is_file() {
            // 아직 설치 전 (앱 메뉴에서 처음 연 경우 등) → 처음 설정 단계 안내
            self.stack.set_visible_child_name("home");
            self.toast("처음 설정이 필요합니다. 아래 단계를 따라 주세요");
            return;
        }
        self.warn_vulkan32();
        if cfg.proton_path().is_none() {
            *self.pending.borrow_mut() = Some(Pending::Launch);
            self.toast("Proton-CachyOS를 먼저 받은 뒤 실행합니다");
            self.download_proton();
            return;
        }
        let u = self.clone();
        self.with_umu(move || u.spawn_launch());
    }

    fn spawn_launch(self: &Rc<Self>) {
        match runner::spawn_detached() {
            Ok(()) => {
                self.starting_since.set(Some(Instant::now()));
                let msg = if umu::steam_runtime_present() {
                    "시작하는 중…"
                } else {
                    "첫 실행: Steam Runtime 을 내려받는 중일 수 있습니다 (수 분 소요)"
                };
                self.set_progress(true, msg);
                self.launch_btn.set_sensitive(false);
                self.show_prev.set_active(false);
            }
            Err(e) => self.toast(&format!("실행 실패: {e}")),
        }
    }

    fn warn_vulkan32(self: &Rc<Self>) {
        if !sysdeps::userns().ok() {
            // 이 상태로는 Proton 이 시작도 못 함 (bwrap: setting up uid map: Permission denied)
            self.toast_action(
                "Ubuntu 보안 설정(AppArmor) 때문에 실행이 실패할 수 있습니다",
                "명령 복사",
                |u| {
                    u.win.clipboard().set_text(&sysdeps::apparmor_install_cmd());
                    u.toast("명령을 복사했습니다. 터미널에 붙여넣어 실행하세요");
                },
            );
            return;
        }
        if !sysdeps::vulkan32().ok() {
            self.toast_action(
                "32비트 Vulkan이 없어 카카오톡이 '준비 중'에서 멈출 수 있습니다",
                "명령 복사",
                |u| {
                    u.win.clipboard().set_text(sysdeps::VULKAN32_CMD);
                    u.toast("명령을 복사했습니다. 터미널에 붙여넣어 실행하세요");
                },
            );
        }
    }

    fn show_error(&self, heading: &str, body: &str, link: Option<&'static str>) {
        let d = adw::MessageDialog::new(Some(&self.win), Some(heading), Some(body));
        d.add_response("close", "닫기");
        if let Some(url) = link {
            d.add_response("open", "페이지 열기");
            d.set_response_appearance("open", adw::ResponseAppearance::Suggested);
            d.connect_response(Some("open"), move |_, _| util::open_uri(url));
        }
        d.present();
    }

    fn toast_action(
        self: &Rc<Self>,
        msg: &str,
        button: &str,
        action: impl Fn(&Rc<Self>) + 'static,
    ) {
        let t = adw::Toast::builder()
            .title(msg)
            .button_label(button)
            .timeout(8)
            .build();
        let u = self.clone();
        t.connect_button_clicked(move |_| action(&u));
        self.toasts.add_toast(t);
    }

    /// umu-run 이 없으면 (Lutris 복사 → GitHub) 확보한 뒤 `then`
    fn with_umu(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        let cfg = self.cfg.borrow().clone();
        if umu::resolve(&cfg).is_some() || !cfg.umu_run.trim().is_empty() {
            then();
            return;
        }
        if self.umu_busy.replace(true) {
            return;
        }
        self.set_busy(Some("umu-launcher 받는 중…"));
        let u = self.clone();
        bg(
            || {
                umu::ensure_own_copy().and_then(|o| match o {
                    Some(_) => Ok(()),
                    None => umu::download_latest().map(|_| ()),
                })
            },
            move |r| {
                u.umu_busy.set(false);
                u.set_busy(None);
                u.refresh_info();
                match r {
                    Ok(()) => then(),
                    Err(e) => u.show_error(
                        "umu-launcher를 받지 못했습니다",
                        &e,
                        Some(umu::RELEASES_PAGE),
                    ),
                }
            },
        );
    }

    fn set_busy(&self, label: Option<&str>) {
        *self.busy.borrow_mut() = label.map(String::from);
        match label {
            Some(l) => self.set_progress(true, l),
            None => self.set_progress(false, ""),
        }
    }

    /// 사용자가 공식 사이트에서 받은 설치 파일 고르기
    fn pick_installer(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Windows 설치 파일 (*.exe)"));
        filter.add_pattern("*.exe");
        filter.add_pattern("*.EXE");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("카카오톡 설치 파일 선택 (KakaoTalk_Setup.exe)")
            .modal(true)
            .filters(&filters)
            .build();
        if let Some(dl) = glib::user_special_dir(glib::UserDirectory::Downloads) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dl)));
        }
        let u = self.clone();
        dialog.open(Some(&self.win), gio::Cancellable::NONE, move |res| {
            if let Ok(f) = res
                && let Some(p) = f.path()
            {
                let u2 = u.clone();
                u.ensure_stopped("카카오톡 설치", move || u2.start_install(p));
            }
        });
        self.toast_action(
            "설치 파일이 없다면 카카오 공식 사이트에서 Windows용을 받으세요",
            "사이트 열기",
            |_| util::open_uri(KAKAO_PAGE),
        );
    }

    fn start_install(self: &Rc<Self>, setup: std::path::PathBuf) {
        self.warn_vulkan32();
        if self.cfg.borrow().proton_path().is_none() {
            // Proton 을 먼저 받고, 끝나면 이 설치 파일로 이어서 설치
            *self.pending.borrow_mut() = Some(Pending::Install(setup));
            self.toast("Proton-CachyOS를 먼저 받은 뒤 설치를 이어갑니다");
            self.download_proton();
            return;
        }
        let u = self.clone();
        self.with_umu(move || match runner::spawn_installer(&setup) {
            Ok(()) => {
                u.installing.set(true);
                u.starting_since.set(Some(Instant::now()));
                u.show_prev.set_active(false);
                u.set_progress(
                    true,
                    "설치 프로그램 실행 중 — 열린 설치 창의 안내를 끝까지 따르세요 (첫 실행은 Steam Runtime 다운로드로 수 분 소요)",
                );
                u.launch_btn.set_sensitive(false);
            }
            Err(e) => u.toast(&format!("설치 프로그램 실행 실패: {e}")),
        });
    }

    fn download_proton(self: &Rc<Self>) {
        if self.proton_dl_active.replace(true) {
            return;
        }
        self.proton_dl_btn.set_sensitive(false);
        self.proton_dl_row.set_visible(true);
        self.proton_dl_row.set_subtitle("최신 릴리스 확인 중…");
        self.proton_dl_bar.set_fraction(0.0);
        self.set_busy(Some("Proton-CachyOS 받는 중…"));
        let u = self.clone();
        let finish = move |u: &Rc<Self>, r: Result<String, String>| {
            u.proton_dl_active.set(false);
            u.proton_dl_btn.set_sensitive(true);
            u.proton_dl_row.set_visible(false);
            u.set_busy(None);
            match r {
                Ok(name) => {
                    let missing = u.cfg.borrow().proton_path().is_none();
                    if missing {
                        u.edit(|c| c.proton = name.clone());
                    }
                    u.fill_proton_list();
                    u.toast(&if missing {
                        format!("{name} 설치 완료 — 선택됨")
                    } else {
                        format!("{name} 설치 완료 — Proton 목록에서 고를 수 있습니다")
                    });
                    u.refresh_setup();
                    // 기다리던 작업 이어서
                    match u.pending.borrow_mut().take() {
                        Some(Pending::Install(p)) => u.start_install(p),
                        Some(Pending::Launch) => u.launch(),
                        None => {}
                    }
                }
                Err(e) => {
                    u.pending.borrow_mut().take();
                    u.show_error(
                        "Proton-CachyOS를 받지 못했습니다",
                        &e,
                        Some("https://github.com/CachyOS/proton-cachyos/releases"),
                    );
                }
            }
        };
        bg(proton::latest_asset, move |r| {
            let asset = match r {
                Ok(a) => a,
                Err(e) => return finish(&u, Err(e)),
            };
            let total = asset.size.max(1);
            let part = asset.part_path();
            let name = asset.name.clone();
            let done = Rc::new(Cell::new(false));
            let (u2, d2) = (u.clone(), done.clone());
            glib::timeout_add_local(Duration::from_millis(300), move || {
                if d2.get() {
                    return glib::ControlFlow::Break;
                }
                let cur = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
                let text = if cur >= total {
                    "압축 푸는 중…".to_string()
                } else {
                    format!("{name} — {} / {} MB", cur / 1_000_000, total / 1_000_000)
                };
                u2.proton_dl_bar.set_fraction(cur as f64 / total as f64);
                u2.proton_dl_row.set_subtitle(&text);
                *u2.busy.borrow_mut() = Some(format!("Proton-CachyOS 받는 중: {text}"));
                glib::ControlFlow::Continue
            });
            let u3 = u.clone();
            bg(
                move || proton::install_asset(&asset),
                move |r| {
                    done.set(true);
                    finish(&u3, r);
                },
            );
        });
    }

    fn kill(self: &Rc<Self>, then: Option<Box<dyn FnOnce()>>) {
        if self.killing.replace(true) {
            return;
        }
        let cfg = self.cfg.borrow().clone();
        self.kill_btn.set_sensitive(false);
        self.toast("종료하는 중…");
        let u = self.clone();
        bg(
            move || util::kill_prefix(&cfg.prefix_path(), cfg.proton_path().as_deref()),
            move |r| {
                // 종료 시도가 끝나면 버튼을 다시 쓸 수 있게 (실패해도 다시 누를 수 있어야 함)
                u.killing.set(false);
                u.kill_btn.set_sensitive(true);
                u.starting_since.set(None);
                u.installing.set(false);
                u.refresh_state();
                match r {
                    Ok(()) => {
                        if let Some(f) = then {
                            f();
                        } else {
                            u.toast("카카오톡을 종료했습니다");
                        }
                    }
                    Err(e) => u.toast(&e),
                }
            },
        );
    }

    /// 카카오톡이 꺼져 있을 때만 `action` 실행. 켜져 있으면 종료할지 묻는다.
    fn ensure_stopped(self: &Rc<Self>, what: &str, action: impl FnOnce() + 'static) {
        let prefix = self.cfg.borrow().prefix_path();
        if util::prefix_procs(&prefix).is_empty() {
            action();
            return;
        }
        let d = adw::MessageDialog::new(
            Some(&self.win),
            Some("카카오톡이 실행 중입니다"),
            Some(&format!(
                "{what}은(는) 카카오톡과 wineserver가 완전히 종료된 뒤에만 안전합니다.\n(실행 중에 레지스트리를 바꾸면 종료 시 덮어써집니다)\n\n강제 종료하고 계속할까요?"
            )),
        );
        d.add_response("cancel", "취소");
        d.add_response("kill", "종료 후 계속");
        d.set_response_appearance("kill", adw::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        d.set_close_response("cancel");
        let u = self.clone();
        let action = RefCell::new(Some(action));
        d.connect_response(None, move |_, resp| {
            if resp == "kill"
                && let Some(a) = action.borrow_mut().take()
            {
                u.kill(Some(Box::new(a)));
            }
        });
        d.present();
    }

    fn repair(self: &Rc<Self>, fixes: Vec<&'static registry::Fix>, with_font: bool) {
        let u = self.clone();
        self.ensure_stopped("레지스트리 수정", move || {
            let cfg = u.cfg.borrow().clone();
            let u2 = u.clone();
            bg(
                move || {
                    let mut msgs = Vec::new();
                    match registry::apply(&cfg.prefix_path(), &fixes) {
                        Ok(0) => msgs.push("레지스트리: 이미 적용됨".to_string()),
                        Ok(n) => msgs.push(format!("레지스트리 {n}개 항목 수정")),
                        Err(e) => msgs.push(e),
                    }
                    if with_font {
                        match fonts::install(&cfg.font_file) {
                            Ok(_) => msgs.push("폰트 최신".into()),
                            Err(e) => msgs.push(format!("폰트: {e}")),
                        }
                    }
                    msgs.join(" · ")
                },
                move |m| {
                    u2.toast(&m);
                    u2.refresh_checks();
                },
            );
        });
    }

    // ───────── 상태 갱신 ─────────

    fn set_progress(&self, on: bool, label: &str) {
        self.progress.set_visible(on);
        self.progress_label.set_visible(on && !label.is_empty());
        self.progress_label.set_label(label);
    }

    fn refresh_state(&self) {
        let prefix = self.cfg.borrow().prefix_path();
        let mut st = util::run_state(&prefix);
        let starting = self.starting_since.get();
        // 분리 실행 직후 프로세스가 아직 안 보이는 구간
        if st == RunState::Stopped
            && let Some(t) = starting
        {
            if t.elapsed() < Duration::from_secs(8) {
                st = RunState::Starting;
            } else {
                self.starting_since.set(None);
            }
        }
        if matches!(st, RunState::Running | RunState::Crashed) {
            self.starting_since.set(None);
        }
        let prev = self.last_state.replace(Some(st));
        let installed = self.cfg.borrow().exe_path().is_file();
        // 설치 프로그램이 마지막에 카카오톡을 바로 띄운 경우 → 설치 완료로 처리
        if self.installing.get() && st == RunState::Running && installed {
            self.installing.set(false);
            self.refresh_info();
            self.toast("카카오톡 설치 완료!");
        }
        let installing = self.installing.get();
        let busy = self.busy.borrow().is_some();
        let (title, desc) = match st {
            RunState::Stopped if busy => ("준비 중…", "필요한 구성 요소를 받는 중입니다"),
            RunState::Stopped if !installed => (
                "설치 필요",
                "공식 사이트에서 받은 Windows용 설치 파일을 선택하세요",
            ),
            RunState::Starting | RunState::Running if installing => {
                ("설치 중…", "설치 창이 열립니다. 안내를 끝까지 진행하세요")
            }
            RunState::Stopped => (
                "꺼져 있음",
                "실행 버튼을 누르거나 앱 메뉴에서 카카오톡을 여세요",
            ),
            RunState::Starting => ("준비 중…", "umu / Proton 이 카카오톡을 띄우는 중입니다"),
            RunState::Running => ("실행 중", "KakaoTalkUI.exe 가 실행되고 있습니다"),
            RunState::Crashed => (
                "크래시 감지",
                "CrashReporter 가 떴습니다. 로그 탭을 확인하세요",
            ),
        };
        self.status_title.set_label(title);
        self.status_desc.set_label(desc);
        self.launch_btn.set_label(if installed {
            "실행"
        } else {
            "설치 파일 선택…"
        });
        self.launch_btn.set_sensitive(st == RunState::Stopped);
        self.kill_btn.set_visible(st != RunState::Stopped);
        self.kill_btn.set_sensitive(!self.killing.get());
        self.refresh_setup();
        if st == RunState::Starting {
            if !self.progress.is_visible() {
                self.set_progress(true, "");
            }
            let l = self.last_line.borrow().clone();
            if !l.is_empty() {
                self.progress_label.set_label(&l);
                self.progress_label.set_visible(true);
            }
        } else if let Some(b) = self.busy.borrow().clone() {
            self.set_progress(true, &b);
        } else if self.progress.is_visible() {
            self.set_progress(false, "");
        }
        if prev != Some(st) {
            if prev.is_some() && st == RunState::Stopped && installing {
                self.installing.set(false);
                self.refresh_info();
                if installed {
                    self.toast("카카오톡 설치 완료! 실행 버튼으로 시작하세요");
                } else {
                    self.toast("설치가 끝나지 않았습니다. 로그 탭을 확인하세요");
                }
            }
            if prev.is_some() && st == RunState::Stopped {
                // 실행이 끝났다 → 바로가기/점검 상태 갱신
                self.refresh_checks();
            }
            if st == RunState::Crashed {
                self.stack.set_visible_child_name("logs");
            }
        }
    }

    fn refresh_setup(&self) {
        let cfg = self.cfg.borrow().clone();
        let vk = sysdeps::vulkan32();
        let umu_ok = umu::resolve(&cfg).is_some();
        let proton_ok = cfg.proton_path().is_some();
        let kakao_ok = cfg.exe_path().is_file();
        let font_ok = fonts::installed(&cfg.font_file).is_some();

        let set = |(r, i, b): &(adw::ActionRow, gtk::Image, gtk::Button), ok: bool, text: &str| {
            set_status(i, ok);
            r.set_subtitle(text);
            b.set_visible(!ok);
        };
        let userns = sysdeps::userns();
        set(&self.step_userns, userns.ok(), userns.summary());
        // 제한 없는 시스템(대부분의 배포판, 예전 Ubuntu)에는 이 단계를 아예 보이지 않음
        self.step_userns
            .0
            .set_visible(userns != sysdeps::Userns::Unrestricted);
        set(&self.step_vk, vk.ok(), vk.summary());
        set(
            &self.step_umu,
            umu_ok,
            if umu_ok {
                "준비됨"
            } else if self.umu_busy.get() {
                "받는 중…"
            } else {
                "없음 — 설치·실행할 때 GitHub에서 자동으로 받습니다"
            },
        );
        set(
            &self.step_proton,
            proton_ok,
            &if proton_ok {
                cfg.proton.clone()
            } else if self.proton_dl_active.get() {
                "받는 중… (설정 탭에서 진행률 확인)".into()
            } else {
                "없음 — 설치·실행할 때 자동으로 받습니다".into()
            },
        );
        set(
            &self.step_kakao,
            kakao_ok,
            if kakao_ok {
                "설치됨"
            } else {
                "카카오 공식 사이트(🌐)에서 Windows용 설치 파일을 받아 선택하세요"
            },
        );
        set(
            &self.step_font,
            font_ok,
            if font_ok {
                "설치됨"
            } else {
                "없으면 이모지가 네모로 보일 수 있습니다"
            },
        );
        // 다운로드 중에는 버튼 숨김
        if self.umu_busy.get() {
            self.step_umu.2.set_visible(false);
        }
        if self.proton_dl_active.get() {
            self.step_proton.2.set_visible(false);
        }
        self.setup_group
            .set_visible(!(userns.ok() && vk.ok() && umu_ok && proton_ok && kakao_ok));
    }

    fn fill_proton_list(&self) {
        let list = proton::scan();
        let mut names: Vec<String> = list.iter().map(|(n, _)| n.clone()).collect();
        let wanted = self.cfg.borrow().proton.clone();
        let sel = proton::pick_default(&list, &wanted);
        if names.is_empty() || (!wanted.is_empty() && !names.contains(&wanted) && sel.is_none()) {
            names.insert(0, wanted.clone());
        }
        let labels: Vec<String> = names
            .iter()
            .map(|n| {
                if list.iter().any(|(m, _)| m == n) {
                    n.clone()
                } else {
                    format!("{n} (없음)")
                }
            })
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        *self.proton_names.borrow_mut() = names.clone();
        self.proton_row
            .set_model(Some(&gtk::StringList::new(&refs)));
        let target = sel.unwrap_or(wanted);
        if let Some(i) = names.iter().position(|n| *n == target) {
            self.proton_row.set_selected(i as u32);
        }
        if target != self.cfg.borrow().proton {
            self.edit(|c| c.proton = target.clone());
            self.toast(&format!(
                "설정한 Proton이 없어 {target} 을(를) 선택했습니다"
            ));
        }
        self.refresh_info();
    }

    fn set_font_names(&self, mut names: Vec<String>) {
        let cur = self.cfg.borrow().font_file.clone();
        if !names.contains(&cur) {
            names.insert(0, cur.clone());
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        *self.font_names.borrow_mut() = names.clone();
        self.font_row.set_model(Some(&gtk::StringList::new(&refs)));
        if let Some(i) = names.iter().position(|n| *n == cur) {
            self.font_row.set_selected(i as u32);
        }
    }

    fn fill_gpu_list(&self) {
        let cur = self.cfg.borrow().gpu.clone();
        let mut opts: Vec<(String, String)> = vec![
            (gpu::MODE_AUTO.into(), "자동 (권장)".into()),
            (gpu::MODE_NONE.into(), "필터 없음".into()),
        ];
        for g in &self.gpus {
            if let Some(f) = g.vendor.filter()
                && !opts.iter().any(|(v, _)| v == f)
            {
                opts.push((f.into(), format!("{f}만 사용")));
            }
        }
        if !cur.is_empty() && !opts.iter().any(|(v, _)| *v == cur) {
            opts.push((cur.clone(), format!("직접 지정: {cur}")));
        }
        let labels: Vec<&str> = opts.iter().map(|(_, l)| l.as_str()).collect();
        self.gpu_row.set_model(Some(&gtk::StringList::new(&labels)));
        let idx = opts.iter().position(|(v, _)| *v == cur).unwrap_or(0);
        *self.gpu_values.borrow_mut() = opts.into_iter().map(|(v, _)| v).collect();
        self.gpu_row.set_selected(idx as u32);
        self.refresh_gpu();
    }

    fn refresh_gpu(&self) {
        let cfg = self.cfg.borrow().clone();
        let vk = self.vk_devices.borrow().clone();
        let c = gpu::resolve(
            &cfg.gpu,
            cfg.env.get(gpu::ENV_NAME).map(String::as_str),
            &self.gpus,
            &vk,
        );
        let text = format!(
            "{} → {}",
            c.reason,
            match &c.filter {
                Some(f) => format!("{}={f}", gpu::ENV_NAME),
                None => "필터 없음".into(),
            }
        );
        self.gpu_row.set_subtitle(&text);
        self.info_gpu.set_subtitle(&text);
        let mut lines: Vec<String> = self.gpus.iter().map(|g| g.describe()).collect();
        if lines.is_empty() {
            lines.push("/sys/class/drm 에서 GPU를 찾지 못함".into());
        }
        if vk.is_empty() {
            lines.push("Vulkan 장치 이름: vulkaninfo 없음 (sudo apt install vulkan-tools 로 더 정확히 확인)".into());
        } else {
            lines.push(format!("Vulkan: {}", vk.join(" / ")));
        }
        self.gpu_info.set_subtitle(&lines.join("\n"));
    }

    fn refresh_info(&self) {
        let cfg = self.cfg.borrow().clone();
        let umu_path = umu::resolve(&cfg);
        let umu_text = match &umu_path {
            Some(p) if p.starts_with(umu::own_dir()) => format!("{} (자체 사본)", p.display()),
            Some(p) if p.starts_with(umu::lutris_dir()) => {
                format!("{} (Lutris — 실행 시 자체 경로로 복사됨)", p.display())
            }
            Some(p) => p.display().to_string(),
            None => "없음 — 실행 시 GitHub에서 받습니다".into(),
        };
        self.info_umu.set_subtitle(&umu_text);
        self.umu_row.set_subtitle(&umu_text);
        let pp = cfg.proton_path();
        self.info_proton.set_subtitle(&match &pp {
            Some(_) => format!("{}\n{}", cfg.proton, proton::hint(&cfg.proton)),
            None => format!("{} — 찾을 수 없음", cfg.proton),
        });
        self.proton_row.set_subtitle(if pp.is_some() {
            proton::hint(&cfg.proton)
        } else {
            "⚠ 설치되지 않음 — 오른쪽 위 '최신 Proton-CachyOS 받기'를 누르세요"
        });
        self.info_rt.set_subtitle(if umu::steam_runtime_present() {
            "준비됨 (~/.local/share/umu)"
        } else {
            "없음 — 첫 실행 때 umu가 내려받습니다 (수 분)"
        });
        let exe = std::env::current_exe().unwrap_or_default();
        let exec = match desktop::current_exec() {
            Some(e) if desktop::points_to(&exe) => format!("✓ {e}"),
            Some(e) => format!("현재: {e}"),
            None => "없음".into(),
        };
        let icon = match crate::icon::installed() {
            Some(p) => format!("아이콘: {}", p.display()),
            None => "아이콘: 없음 — 카카오톡 설치 후 '교체'를 누르면 exe에서 추출 (icoutils 필요)"
                .into(),
        };
        self.desktop_row.set_subtitle(&format!("{exec}\n{icon}"));
        let kexe = cfg.exe_path();
        self.info_kakao.set_subtitle(&if kexe.is_file() {
            format!("설치됨 · {}", kexe.display())
        } else {
            "설치되지 않음".into()
        });
    }

    fn refresh_checks(&self) {
        let (icon_ok, icon_text) = crate::icon::status();
        set_status(&self.icon_state, icon_ok);
        self.icon_row.set_subtitle(&if icon_ok {
            icon_text
        } else {
            format!(
                "이럴 때: 앱 메뉴에서 카카오톡이 말풍선 모양 기본 아이콘으로 보일 때\n{icon_text}"
            )
        });
        let cfg = self.cfg.borrow().clone();
        let prefix = cfg.prefix_path();
        match registry::check(&prefix) {
            Ok(res) => {
                for (f, r, icon) in self.fix_rows.borrow().iter() {
                    let ok = res.iter().any(|(g, ok)| g.id == f.id && *ok);
                    icon.set_icon_name(Some(if ok {
                        "emblem-ok-symbolic"
                    } else {
                        "dialog-warning-symbolic"
                    }));
                    icon.remove_css_class(if ok { "warning" } else { "success" });
                    icon.add_css_class(if ok { "success" } else { "warning" });
                    r.set_subtitle(&format!(
                        "{}\n{}",
                        f.why,
                        if ok {
                            "✓ 적용됨"
                        } else {
                            "아직 적용 안 됨 — '적용' 또는 '원클릭 수리'를 누르세요"
                        }
                    ));
                }
            }
            Err(e) => {
                for (_, r, _) in self.fix_rows.borrow().iter() {
                    r.set_subtitle(&e);
                }
            }
        }
        // [Software\\Wine] Version=win10 이 있으면 실제로는 Windows 10 으로 동작
        let win10_set = registry::check(&prefix).is_ok_and(|v| {
            v.iter()
                .any(|(f, ok)| f.id == registry::FIX_WIN10_PREFIX.id && *ok)
        });
        self.winbuild_row
            .set_subtitle(&match registry::windows_build(&prefix) {
                _ if win10_set => "Windows 10으로 맞춰져 있습니다".to_string(),
                Some(b) if b.parse::<u32>().is_ok_and(|n| n >= 22000) => format!(
                    "Windows 11 (빌드 {b}) — 창 양옆 하얀 줄이 생길 수 있어요. 위 'Windows 10으로 맞추기'를 적용하세요"
                ),
                Some(b) => format!("빌드 {b}"),
                None => "아직 모름 — 카카오톡을 설치하면 표시됩니다".into(),
            });
        if cfg.proton != cfg.last_proton && !cfg.last_proton.is_empty() {
            self.winbuild_row.set_subtitle(&format!(
                "Proton이 바뀌었습니다 ({} → {}). 다음에 카카오톡을 켤 때 설정을 다시 점검합니다",
                cfg.last_proton, cfg.proton
            ));
        }
        self.font_status
            .set_subtitle(&match fonts::installed(&cfg.font_file) {
                Some((size, t)) => format!("설치됨 · {size} bytes · {t}"),
                None => "설치되지 않음".into(),
            });
    }

    // ───────── 로그 ─────────

    fn reload_log(&self) {
        let path = if self.show_prev.is_active() {
            runner::prev_log_path()
        } else {
            runner::log_path()
        };
        let text = fs::read(&path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        self.tail_offset.set(text.len() as u64);
        let shown = if text.is_empty() {
            "(로그 없음)".to_string()
        } else {
            util::mask_secrets(&text)
        };
        self.log_buffer.set_text(&shown);
        self.scroll_log_end();
        self.update_findings(&text);
    }

    fn tail_log(&self) {
        if self.show_prev.is_active() {
            return;
        }
        let path = runner::log_path();
        let Ok(meta) = fs::metadata(&path) else {
            return;
        };
        let len = meta.len();
        let off = self.tail_offset.get();
        if len < off {
            // 새 실행으로 교체됨(rotate / 마스킹 재작성)
            self.reload_log();
            return;
        }
        if len == off {
            return;
        }
        let Ok(mut f) = fs::File::open(&path) else {
            return;
        };
        if f.seek(SeekFrom::Start(off)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        let _ = f.take(len - off).read_to_end(&mut buf);
        // 줄 단위로만 소비 (UTF-8 경계 보호)
        let Some(cut) = buf.iter().rposition(|b| *b == b'\n') else {
            return;
        };
        buf.truncate(cut + 1);
        self.tail_offset.set(off + buf.len() as u64);
        let chunk = util::mask_secrets(&String::from_utf8_lossy(&buf));
        if let Some(l) = chunk.lines().rev().find(|l| !l.trim().is_empty()) {
            *self.last_line.borrow_mut() = l.chars().take(160).collect();
        }
        let mut end = self.log_buffer.end_iter();
        if off == 0 {
            self.log_buffer.set_text("");
            end = self.log_buffer.end_iter();
        }
        self.log_buffer.insert(&mut end, &chunk);
        self.scroll_log_end();
        if chunk.contains(runner::END_MARKER)
            || chunk.to_ascii_lowercase().contains("crashreporter")
        {
            let b = &self.log_buffer;
            let all = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
            self.update_findings(&all);
        }
    }

    fn scroll_log_end(&self) {
        let mark = self
            .log_buffer
            .create_mark(None, &self.log_buffer.end_iter(), false);
        self.log_view.scroll_mark_onscreen(&mark);
        self.log_buffer.delete_mark(&mark);
    }

    fn update_findings(&self, text: &str) {
        while let Some(c) = self.findings_box.first_child() {
            self.findings_box.remove(&c);
        }
        let findings = runner::analyze(text);
        self.findings_box.set_visible(!findings.is_empty());
        for f in &findings {
            let r = row(&format!("[{}] {}", f.level.label(), f.title), &f.detail);
            r.add_prefix(&gtk::Image::from_icon_name(match f.level {
                Level::Error => "dialog-error-symbolic",
                Level::Warning => "dialog-warning-symbolic",
                Level::Info => "dialog-information-symbolic",
            }));
            self.findings_box.append(&r);
        }
        let err = findings.iter().find(|f| f.level == Level::Error);
        match err {
            Some(f) => {
                let which = if self.show_prev.is_active() {
                    "이전 실행"
                } else {
                    "마지막 실행"
                };
                let t = format!("{which}에서 문제 감지: {}", f.title);
                self.log_banner.set_title(&t);
                self.log_banner.set_revealed(true);
                self.home_banner.set_title(&t);
                self.home_banner.set_revealed(true);
            }
            None => {
                self.log_banner.set_revealed(false);
                self.home_banner.set_revealed(false);
            }
        }
    }
}
