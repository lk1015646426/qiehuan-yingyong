mod commands;
pub mod error;
mod models;
mod modules;
mod utils;

use modules::config::CloseWindowBehavior;
use modules::logger;
use std::sync::OnceLock;
#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;
use tauri::RunEvent;
use tauri::WindowEvent;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use tracing::info;

/// 全局 AppHandle 存储
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 获取全局 AppHandle
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

#[cfg(test)]
mod tests {
    use super::should_hide_startup_minimized_window;
    use crate::modules::config::UserConfig;

    #[test]
    fn startup_minimized_does_not_hide_when_disabled() {
        let mut config = UserConfig::default();
        config.startup_minimized = false;
        config.hide_dock_icon = true;

        assert!(!should_hide_startup_minimized_window(&config, true));
    }

    #[test]
    fn startup_minimized_hides_on_macos_when_dock_icon_is_hidden() {
        let mut config = UserConfig::default();
        config.startup_minimized = true;
        config.hide_dock_icon = true;

        assert!(should_hide_startup_minimized_window(&config, true));
    }

    #[test]
    fn startup_minimized_does_not_hide_when_dock_icon_is_available() {
        let mut config = UserConfig::default();
        config.startup_minimized = true;
        config.hide_dock_icon = false;

        assert!(!should_hide_startup_minimized_window(&config, true));
    }

    #[test]
    fn startup_minimized_does_not_wait_before_hiding_window() {
        let source = include_str!("lib.rs");
        let delayed_startup_hide = concat!(
            "std::thread::sleep",
            "(std::time::Duration::from_millis(300))"
        );

        assert!(!source.contains(delayed_startup_hide));
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn raise_process_file_descriptor_limit() {
    const TARGET_NOFILE_LIMIT: libc::rlim_t = 4096;

    unsafe {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
            logger::log_warn(&format!(
                "[Startup] 读取进程文件句柄上限失败: {}",
                std::io::Error::last_os_error()
            ));
            return;
        }

        let target = if limit.rlim_max == libc::RLIM_INFINITY {
            TARGET_NOFILE_LIMIT
        } else {
            TARGET_NOFILE_LIMIT.min(limit.rlim_max)
        };
        if target <= limit.rlim_cur || target == 0 {
            return;
        }

        let previous = limit.rlim_cur;
        limit.rlim_cur = target;
        if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == 0 {
            logger::log_info(&format!(
                "[Startup] 已提升进程文件句柄软限制: {} -> {}",
                previous, target
            ));
        } else {
            logger::log_warn(&format!(
                "[Startup] 提升进程文件句柄软限制失败: {} -> {}, error={}",
                previous,
                target,
                std::io::Error::last_os_error()
            ));
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn raise_process_file_descriptor_limit() {}

fn should_hide_startup_minimized_window(
    config: &modules::config::UserConfig,
    is_macos: bool,
) -> bool {
    config.startup_minimized && is_macos && config.hide_dock_icon
}

fn apply_startup_minimized(app: &tauri::AppHandle) {
    let config = modules::config::get_user_config();
    if !config.startup_minimized {
        return;
    }

    let should_hide = should_hide_startup_minimized_window(&config, cfg!(target_os = "macos"));
    let Some(window) = app.get_webview_window("main") else {
        logger::log_warn("[Window] 启动后自动最小化失败: main window not found");
        return;
    };

    let (result, action_label) = if should_hide {
        (window.hide(), "隐藏")
    } else {
        (window.minimize(), "最小化")
    };

    match result {
        Ok(()) => logger::log_info(&format!("[Window] 启动后已自动{}主窗口", action_label)),
        Err(err) => logger::log_warn(&format!("[Window] 启动后自动最小化失败: {}", err)),
    }
}

#[cfg(target_os = "macos")]
fn apply_macos_activation_policy(app: &tauri::AppHandle) {
    let config = modules::config::get_user_config();
    let (policy, dock_visible, policy_label) = if config.hide_dock_icon {
        (ActivationPolicy::Accessory, false, "hidden")
    } else {
        (ActivationPolicy::Regular, true, "visible")
    };

    if let Err(err) = app.set_activation_policy(policy) {
        logger::log_warn(&format!("[Window] 设置 macOS 激活策略失败: {}", err));
        return;
    }

    if let Err(err) = app.set_dock_visibility(dock_visible) {
        logger::log_warn(&format!("[Window] 设置 macOS Dock 可见性失败: {}", err));
    }

    if dock_visible {
        let _ = app.show();
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
        }
    }

    info!("[Window] 已应用 macOS Dock 图标策略: {}", policy_label);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logger::init_logger();
    modules::diagnostics::install_panic_hook();
    modules::diagnostics::start_frontend_ready_watchdog();
    raise_process_file_descriptor_limit();
    // 启动时先加载一次配置，确保进程级代理环境与用户设置同步。
    let _ = modules::config::get_user_config();

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            logger::log_info("[Linux] 设置 WEBKIT_DISABLE_DMABUF_RENDERER=1");
        }
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            logger::log_info(&format!(
                "[SingleInstance] 收到唤起请求: arg_count={}",
                args.len()
            ));
            if let Err(err) = modules::floating_card_window::show_main_window(app) {
                logger::log_warn(&format!("[Window] 单实例唤起恢复主窗口失败: {}", err));
            }
        }))
        .setup(|app| {
            info!("Cockpit Tools 启动...");
            let current_exe = std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|err| format!("unknown: {}", err));
            let build_mode = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            logger::log_info(&format!(
                "[Startup] 启动诊断: marker=tray-diagnostics-v1, version={}, mode={}, exe={}",
                env!("CARGO_PKG_VERSION"),
                build_mode,
                current_exe
            ));

            // 存储全局 AppHandle
            let _ = APP_HANDLE.set(app.handle().clone());

            if let Err(err) = modules::app_lifecycle::install_system_shutdown_listener() {
                logger::log_warn(&format!("[Lifecycle] 安装系统关机监听失败: {}", err));
            }

            // 启动时清理 WebKit LocalStorage WAL，防止无限膨胀
            std::thread::spawn(|| {
                modules::webkit_cache_maintenance::checkpoint_webkit_localstorage();
            });

            // 当前主线不再使用 platform-packages；启动时回收旧版本遗留的孤儿 adapter。
            std::thread::spawn(|| {
                match modules::process::close_orphaned_legacy_platform_adapter_processes(5) {
                    Ok(0) => {}
                    Ok(count) => logger::log_info(&format!(
                        "[LegacyAdapterCleanup] 已清理旧平台 adapter 进程: count={}",
                        count
                    )),
                    Err(err) => logger::log_warn(&format!(
                        "[LegacyAdapterCleanup] 清理旧平台 adapter 进程失败: {}",
                        err
                    )),
                }
            });

            // 初始化 Process + Autostart 插件（Updater 已于阶段 8 移除）
            #[cfg(desktop)]
            {
                app.handle().plugin(tauri_plugin_process::init())?;
                app.handle().plugin(tauri_plugin_autostart::init(
                    tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                    None::<Vec<&'static str>>,
                ))?;
                info!("[Plugin] Tauri Process + Autostart 插件已初始化");
            }

            // 启动时同步设置合并（移至后台线程，不阻塞窗口显示）
            std::thread::spawn(|| {
                let current_config = modules::config::get_user_config();
                if let Some(merged_language) = modules::sync_settings::merge_setting_on_startup(
                    "language",
                    &current_config.language,
                    None,
                ) {
                    info!(
                        "[SyncSettings] 启动时合并语言设置: {} -> {}",
                        current_config.language, merged_language
                    );
                    if let Err(e) = modules::config::patch_user_config(|config| {
                        config.language = merged_language;
                        Ok(())
                    }) {
                        logger::log_error(&format!("[SyncSettings] 保存合并后的配置失败: {}", e));
                    }
                }
            });


            tauri::async_runtime::spawn(async {
                modules::trae_oauth::restore_pending_oauth_listener();
            });

            modules::work_cn_session_watcher::ensure_started(app.handle().clone());


            #[cfg(target_os = "macos")]
            apply_macos_activation_policy(&app.handle());

            #[cfg(any(windows, target_os = "linux"))]
            {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Err(err) = app_handle.deep_link().register_all() {
                        logger::log_warn(&format!("[DeepLink] register_all 失败: {}", err));
                    } else {
                        logger::log_info("[DeepLink] register_all 已完成");
                    }
                });
            }


            // 创建骨架托盘（无账号文件 I/O，秒出）
            if let Err(e) = modules::tray::create_tray_skeleton(app.handle()) {
                logger::log_error(&format!("[Tray] 创建骨架托盘失败: {}", e));
            }

            #[cfg(target_os = "macos")]
            {
                let tray_app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    if let Err(err) = modules::tray::apply_tray_icon_style(&tray_app_handle) {
                        logger::log_warn(&format!(
                            "[Tray] macOS 启动后重应用菜单栏图标样式失败: {}",
                            err
                        ));
                    }
                });
            }

            // 后台线程加载完整托盘菜单（含账号数据）
            let tray_app_handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = modules::tray::update_tray_menu(&tray_app_handle) {
                    logger::log_error(&format!("[Tray] 后台更新托盘菜单失败: {}", e));
                }
            });

            if let Err(err) =
                modules::floating_card_window::show_floating_card_window_on_startup(&app.handle())
            {
                logger::log_warn(&format!("[FloatingCard] 启动时显示悬浮卡片失败: {}", err));
            }

            // Restore last main-window size/position before optional startup minimize (#948 / #1132).
            if let Some(main) = app.get_webview_window("main") {
                modules::main_window_state::restore_to_window(&main);
            }

            apply_startup_minimized(&app.handle());

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                if window.label() != "main" {
                    return;
                }
                // Always snapshot geometry before close / tray-destroy / quit.
                modules::main_window_state::capture_and_save_from_window_handle(window);
                let config = modules::config::get_user_config();

                match config.close_behavior {
                    CloseWindowBehavior::Minimize => {
                        api.prevent_close();
                        // Full #686 behavior: destroy main WebView, keep tray process alive.
                        if let Err(err) =
                            modules::floating_card_window::destroy_main_window_to_tray(window)
                        {
                            modules::logger::log_warn(&format!(
                                "[Window] 销毁主窗口 WebView 失败，回退为隐藏: {}",
                                err
                            ));
                            let _ = window.hide();
                            modules::process_memory::trim_idle_process_memory();
                        }
                        info!("[Window] 窗口已关闭到托盘");
                    }
                    CloseWindowBehavior::Quit => {
                        modules::floating_card_window::request_app_exit();
                        info!("[Window] 用户选择退出应用");
                        window.app_handle().exit(0);
                    }
                    CloseWindowBehavior::Ask => {
                        api.prevent_close();
                        let _ = window.emit("window:close_requested", ());
                        info!("[Window] 等待用户选择关闭行为");
                    }
                }
            }
            WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                if window.label() == "main" {
                    modules::main_window_state::capture_and_save_from_window_handle_debounced(
                        window,
                    );
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            // Work CN Commands
            commands::work_cn::get_work_cn_installation,
            commands::work_cn::import_current_work_cn_account,
            commands::work_cn::list_work_cn_accounts,
            commands::work_cn::validate_work_cn_account,
            commands::work_cn::switch_work_cn_account,
            commands::work_cn::get_work_cn_credits,
            commands::work_cn::get_work_cn_session_watch_status,
            commands::work_cn::clear_work_cn_credentials,
            // Work CN GitHub Commands
            commands::work_cn_github::get_work_cn_github_config,
            commands::work_cn_github::save_work_cn_github_config,
            commands::work_cn_github::github_cli_status,
            commands::work_cn_github::sync_work_cn_github_account,
            // Trae Instance Commands
            commands::trae_instance::trae_get_instance_defaults,
            commands::trae_instance::trae_list_instances,
            commands::trae_instance::trae_create_instance,
            commands::trae_instance::trae_update_instance,
            commands::trae_instance::trae_delete_instance,
            commands::trae_instance::trae_start_instance,
            commands::trae_instance::trae_stop_instance,
            commands::trae_instance::trae_open_instance_window,
            commands::trae_instance::trae_close_all_instances,
            // System Commands
            commands::system::open_data_folder,
            commands::system::open_local_path,
            commands::system::save_text_file,
            commands::system::get_downloads_dir,
            commands::system::get_auto_backup_settings,
            commands::system::save_auto_backup_settings,
            commands::system::update_auto_backup_last_run,
            commands::system::write_auto_backup_file,
            commands::system::read_auto_backup_file,
            commands::system::copy_auto_backup_file,
            commands::system::list_auto_backup_files,
            commands::system::delete_auto_backup_file,
            commands::system::cleanup_auto_backup_files,
            commands::system::open_auto_backup_dir,
            commands::system::get_network_config,
            commands::system::save_network_config,
            commands::system::get_available_terminals,
            commands::system::get_diagnostics_config,
            commands::system::save_diagnostics_config,
            commands::system::diagnostics_frontend_stage,
            commands::system::diagnostics_frontend_ready,
            commands::system::diagnostics_capture_event,
            commands::system::get_general_config,
            commands::system::patch_general_config,
            commands::system::save_general_config,
            commands::system::save_refresh_interval_config,
            commands::system::handle_window_close,
            commands::system::main_window_take_pending_navigation,
            commands::system::show_floating_card_window,
            commands::system::show_instance_floating_card_window,
            commands::system::get_floating_card_context,
            commands::system::hide_floating_card_window,
            commands::system::hide_current_floating_card_window,
            commands::system::set_floating_card_always_on_top,
            commands::system::set_current_floating_card_window_always_on_top,
            commands::system::set_floating_card_confirm_on_close,
            commands::system::save_floating_card_position,
            commands::system::show_main_window_and_navigate,
            commands::system::open_folder,
            commands::system::delete_corrupted_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        match &event {
            RunEvent::ExitRequested { api, .. } => {
                if modules::floating_card_window::should_keep_alive_after_main_window_destroyed()
                    && !modules::app_lifecycle::is_shutdown_started()
                {
                    api.prevent_exit();
                    modules::logger::log_info("[Window] 主窗口已销毁，应用继续在托盘运行");
                } else {
                    modules::app_lifecycle::begin_shutdown();
                }
            }
            RunEvent::Exit => {
                modules::app_lifecycle::begin_shutdown();
            }
            _ => {}
        }

        #[cfg(target_os = "macos")]
        {
            match event {
                RunEvent::Reopen { .. } => {
                    if let Err(err) = modules::floating_card_window::show_main_window(app_handle) {
                        logger::log_warn(&format!("[Window] Dock 重新打开主窗口失败: {}", err));
                    }
                }
                RunEvent::Opened { urls } => {
                    let args: Vec<String> = urls.iter().map(|url| url.to_string()).collect();
                    logger::log_info(&format!(
                        "[RunEvent] 收到 Opened 事件: url_count={}, urls={:?}",
                        args.len(),
                        args
                    ));
                }
                _ => {}
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app_handle, event);
        }
    });
}
