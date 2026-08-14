//! 极简系统托盘模块（阶段 8 收敛为 TRAE Work CN 单平台）。
//!
//! 仅提供「打开 TRAE Work CN 切换器」与「退出」两项菜单，以及 macOS 菜单栏
//! 图标样式处理。保留 `PlatformId` 仅因 `commands/system.rs` 的通用配置归一化
//! 需要解析 `menu_bar_quota_platform` 字段。

#[cfg(target_os = "macos")]
use tauri::image::Image;
#[cfg(not(target_os = "macos"))]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    Runtime,
};
use tracing::info;

#[cfg(target_os = "macos")]
use crate::modules::config::TrayIconStyle;
use crate::modules::logger;

/// 托盘菜单 ID
pub const TRAY_ID: &str = "main-tray";

#[cfg(target_os = "macos")]
const MACOS_STATUS_ITEM_AUTOSAVE_NAME: &str = "com.jlcodes.cockpit-tools.main-tray";

#[cfg(target_os = "macos")]
const MACOS_TRAY_TEMPLATE_ICON_SIZE: u32 = 36;

#[cfg(target_os = "macos")]
const MACOS_TRAY_TEMPLATE_FALLBACK_RGB: u8 = 225;

/// 平台标识（仅保留用于通用配置字段 `menu_bar_quota_platform` 的归一化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PlatformId {
    Antigravity,
    Codex,
    Claude,
    Zed,
    GitHubCopilot,
    Windsurf,
    Kiro,
    Cursor,
    Grok,
    Codebuddy,
    CodebuddyCn,
    Qoder,
    Zcode,
    Trae,
    TraeSolo,
    TraeCn,
    TraeSoloCn,
    Workbuddy,
}

impl PlatformId {
    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value.trim() {
            "antigravity" => Some(Self::Antigravity),
            "codex" => Some(Self::Codex),
            "claude_manager" | "claude" => Some(Self::Claude),
            "zed" => Some(Self::Zed),
            "github-copilot" => Some(Self::GitHubCopilot),
            "windsurf" => Some(Self::Windsurf),
            "kiro" => Some(Self::Kiro),
            "cursor" => Some(Self::Cursor),
            "grok" => Some(Self::Grok),
            "codebuddy" => Some(Self::Codebuddy),
            "codebuddy_cn" => Some(Self::CodebuddyCn),
            "qoder" => Some(Self::Qoder),
            "zcode" => Some(Self::Zcode),
            "trae" => Some(Self::Trae),
            "trae_solo" => Some(Self::TraeSolo),
            "trae_cn" => Some(Self::TraeCn),
            "trae_solo_cn" => Some(Self::TraeSoloCn),
            "workbuddy" => Some(Self::Workbuddy),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
            Self::Codex => "codex",
            Self::Claude => "claude_manager",
            Self::Zed => "zed",
            Self::GitHubCopilot => "github-copilot",
            Self::Windsurf => "windsurf",
            Self::Kiro => "kiro",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::Codebuddy => "codebuddy",
            Self::CodebuddyCn => "codebuddy_cn",
            Self::Qoder => "qoder",
            Self::Zcode => "zcode",
            Self::Trae => "trae",
            Self::TraeSolo => "trae_solo",
            Self::TraeCn => "trae_cn",
            Self::TraeSoloCn => "trae_solo_cn",
            Self::Workbuddy => "workbuddy",
        }
    }
}

/// 菜单项 ID
pub mod menu_ids {
    pub const SHOW_WINDOW: &str = "show_window";
    pub const QUIT: &str = "quit";
}

#[cfg(target_os = "macos")]
fn build_macos_template_tray_icon() -> Result<Image<'static>, tauri::Error> {
    let source = Image::from_bytes(include_bytes!("../../icons/tray/status-template.png"))?;
    let source_width = source.width();
    let source_height = source.height();
    let source_rgba = source.rgba();
    let target_size = MACOS_TRAY_TEMPLATE_ICON_SIZE;
    let mut target_rgba = Vec::with_capacity((target_size * target_size * 4) as usize);

    for target_y in 0..target_size {
        let src_y_start = target_y * source_height / target_size;
        let src_y_end = ((target_y + 1) * source_height / target_size)
            .max(src_y_start + 1)
            .min(source_height);

        for target_x in 0..target_size {
            let src_x_start = target_x * source_width / target_size;
            let src_x_end = ((target_x + 1) * source_width / target_size)
                .max(src_x_start + 1)
                .min(source_width);

            let mut alpha_sum: u32 = 0;
            let mut sample_count: u32 = 0;
            for src_y in src_y_start..src_y_end {
                for src_x in src_x_start..src_x_end {
                    let index = ((src_y * source_width + src_x) * 4 + 3) as usize;
                    alpha_sum += source_rgba[index] as u32;
                    sample_count += 1;
                }
            }

            let alpha = if sample_count == 0 {
                0
            } else {
                (alpha_sum / sample_count) as u8
            };
            target_rgba.extend_from_slice(&[
                MACOS_TRAY_TEMPLATE_FALLBACK_RGB,
                MACOS_TRAY_TEMPLATE_FALLBACK_RGB,
                MACOS_TRAY_TEMPLATE_FALLBACK_RGB,
                alpha,
            ]);
        }
    }

    Ok(Image::new_owned(target_rgba, target_size, target_size))
}

#[cfg(target_os = "macos")]
fn macos_tray_icon_for_style<'a, R: Runtime>(
    app: &'a tauri::AppHandle<R>,
    style: TrayIconStyle,
) -> Result<(Image<'a>, bool), tauri::Error> {
    match style {
        TrayIconStyle::Template => Ok((build_macos_template_tray_icon()?, true)),
        TrayIconStyle::Color => Ok((
            app.default_window_icon()
                .expect("default window icon should exist")
                .clone(),
            false,
        )),
    }
}

#[cfg(target_os = "macos")]
fn configure_macos_status_item_identity<R: Runtime>(tray: &TrayIcon<R>) {
    let result = tray.with_inner_tray_icon(|tray_icon| {
        let Some(status_item) = tray_icon.ns_status_item() else {
            return "status_item=none".to_string();
        };

        let autosave_name = objc2_foundation::NSString::from_str(MACOS_STATUS_ITEM_AUTOSAVE_NAME);
        status_item.setAutosaveName(Some(&autosave_name));
        status_item.setVisible(true);
        status_item.setLength(objc2_app_kit::NSVariableStatusItemLength);

        let current_autosave_name = status_item.autosaveName().to_string();
        format!(
            "autosave_name={}, visible={}, length={}",
            current_autosave_name,
            status_item.isVisible(),
            status_item.length()
        )
    });

    match result {
        Ok(detail) => logger::log_info(&format!("[Tray] macOS 状态栏项目身份已设置: {}", detail)),
        Err(err) => logger::log_warn(&format!("[Tray] macOS 状态栏项目身份设置失败: {}", err)),
    }
}

#[cfg(target_os = "macos")]
pub fn apply_tray_icon_style<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    let style = crate::modules::config::get_user_config().tray_icon_style;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let (icon, icon_as_template) =
            macos_tray_icon_for_style(app, style).map_err(|err| err.to_string())?;
        let icon_width = icon.width();
        let icon_height = icon.height();
        tray.set_icon(Some(icon)).map_err(|err| err.to_string())?;
        tray.set_icon_as_template(icon_as_template)
            .map_err(|err| err.to_string())?;
        let rect_log = match tray.rect() {
            Ok(Some(rect)) => format!("rect={:?}", rect),
            Ok(None) => "rect=none".to_string(),
            Err(err) => format!("rect_error={}", err),
        };
        logger::log_info(&format!(
            "[Tray] macOS 菜单栏图标样式已应用: style={}, icon={}x{}, template={}, {}",
            style.as_str(),
            icon_width,
            icon_height,
            icon_as_template,
            rect_log
        ));
    }
    Ok(())
}

/// 创建骨架托盘（仅「打开」+「退出」两项，无账号文件 I/O，秒出）。
pub fn create_tray_skeleton<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<TrayIcon<R>, tauri::Error> {
    info!("[Tray] 创建骨架托盘...");

    #[cfg(not(target_os = "macos"))]
    let lang = crate::modules::config::get_user_config().language;

    #[cfg(not(target_os = "macos"))]
    let show_window = MenuItem::with_id(
        app,
        menu_ids::SHOW_WINDOW,
        get_text("show_window", &lang),
        true,
        None::<&str>,
    )?;
    #[cfg(not(target_os = "macos"))]
    let quit = MenuItem::with_id(
        app,
        menu_ids::QUIT,
        get_text("quit", &lang),
        true,
        None::<&str>,
    )?;

    #[cfg(not(target_os = "macos"))]
    let menu = {
        let menu = Menu::new(app)?;
        menu.append(&show_window)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&quit)?;
        menu
    };

    #[cfg(target_os = "macos")]
    let (tray_icon, tray_icon_as_template) = macos_tray_icon_for_style(
        app,
        crate::modules::config::get_user_config().tray_icon_style,
    )?;
    #[cfg(not(target_os = "macos"))]
    let tray_icon = app.default_window_icon().unwrap().clone();

    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icon)
        .show_menu_on_left_click(false)
        .tooltip("TRAE Work CN 账号切换器")
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_event);

    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(tray_icon_as_template);

    #[cfg(not(target_os = "macos"))]
    let builder = builder.menu(&menu);

    let tray = builder.build(app)?;

    #[cfg(target_os = "macos")]
    let _ = tray.set_show_menu_on_left_click(false);
    #[cfg(target_os = "macos")]
    let _ = tray.set_icon_as_template(tray_icon_as_template);
    #[cfg(target_os = "macos")]
    configure_macos_status_item_identity(&tray);

    info!("[Tray] 骨架托盘创建完成");
    Ok(tray)
}

/// 更新托盘菜单（收敛后菜单固定，无需动态账号数据；macOS 下仅刷新状态栏）。
pub fn update_tray_menu<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let lang = crate::modules::config::get_user_config().language;
        let show_window = MenuItem::with_id(
            app,
            menu_ids::SHOW_WINDOW,
            get_text("show_window", &lang),
            true,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        let quit = MenuItem::with_id(
            app,
            menu_ids::QUIT,
            get_text("quit", &lang),
            true,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;

        let menu = Menu::new(app).map_err(|e| e.to_string())?;
        menu.append(&show_window).map_err(|e| e.to_string())?;
        let separator = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
        menu.append(&separator).map_err(|e| e.to_string())?;
        menu.append(&quit).map_err(|e| e.to_string())?;
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
        logger::log_info("[Tray] 托盘菜单已更新");
    }
    Ok(())
}

fn handle_menu_event<R: Runtime>(app: &tauri::AppHandle<R>, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    logger::log_info(&format!("[Tray] 菜单点击: {}", id));

    match id {
        menu_ids::SHOW_WINDOW => {
            if let Err(err) = crate::modules::floating_card_window::show_main_window(app) {
                logger::log_warn(&format!("[Tray] 显示主窗口失败: {}", err));
            }
        }
        menu_ids::QUIT => {
            info!("[Tray] 用户选择退出应用");
            crate::modules::floating_card_window::request_app_exit();
            app.exit(0);
        }
        _ => {}
    }
}

/// 处理托盘图标事件
fn handle_tray_event<R: Runtime>(tray: &TrayIcon<R>, event: TrayIconEvent) {
    match event {
        TrayIconEvent::Click {
            button,
            button_state,
            ..
        } => {
            #[cfg(target_os = "macos")]
            {
                if button == MouseButton::Left {
                    if let Err(err) =
                        crate::modules::floating_card_window::show_main_window(tray.app_handle())
                    {
                        logger::log_warn(&format!("[Tray] 左键恢复主窗口失败: {}", err));
                    }
                    return;
                }
            }

            #[cfg(not(target_os = "macos"))]
            if button == MouseButton::Left && button_state == MouseButtonState::Up {
                if let Err(err) =
                    crate::modules::floating_card_window::show_main_window(tray.app_handle())
                {
                    logger::log_warn(&format!("[Tray] 左键恢复主窗口失败: {}", err));
                }
            }
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            #[cfg(target_os = "macos")]
            {
                return;
            }

            #[cfg(not(target_os = "macos"))]
            if let Err(err) =
                crate::modules::floating_card_window::show_main_window(tray.app_handle())
            {
                logger::log_warn(&format!("[Tray] 双击恢复主窗口失败: {}", err));
            }
        }
        _ => {}
    }
}

/// 获取本地化文本（极简托盘仅保留「打开」与「退出」两项文案）。
#[cfg(not(target_os = "macos"))]
fn get_text(key: &str, lang: &str) -> String {
    let lang = lang.to_ascii_lowercase();
    match (key, lang.as_str()) {
        ("show_window", "zh-cn") | ("show_window", "zh-tw") => "打开 TRAE Work CN 切换器".to_string(),
        ("quit", "zh-cn") | ("quit", "zh-tw") => "退出".to_string(),
        ("show_window", "ja") => "TRAE Work CN 切替器を開く".to_string(),
        ("quit", "ja") => "終了".to_string(),
        ("show_window", _) => "Open TRAE Work CN Switcher".to_string(),
        ("quit", _) => "Quit".to_string(),
        _ => key.to_string(),
    }
}
