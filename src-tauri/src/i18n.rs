use std::env;

use tauri::AppHandle;

use crate::models::AppLocale;
#[cfg(target_os = "macos")]
use crate::models::TrayUsageDisplayMode;
use crate::store::load_store_read_only;

pub(crate) fn detect_system_locale() -> AppLocale {
    let candidates = [
        env::var("LC_ALL").ok(),
        env::var("LC_MESSAGES").ok(),
        env::var("LANG").ok(),
    ];

    for candidate in candidates.into_iter().flatten() {
        let normalized = candidate.to_lowercase();
        if normalized.starts_with("zh") {
            return AppLocale::ZhCn;
        }
        if normalized.starts_with("en") {
            return AppLocale::EnUs;
        }
        if normalized.starts_with("ja") {
            return AppLocale::JaJp;
        }
        if normalized.starts_with("ko") {
            return AppLocale::KoKr;
        }
        if normalized.starts_with("ru") {
            return AppLocale::RuRu;
        }
    }

    AppLocale::default()
}

pub(crate) fn app_locale(app: &AppHandle) -> AppLocale {
    load_store_read_only(app)
        .map(|store| store.settings.locale)
        .unwrap_or_else(|_| detect_system_locale())
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_usage_mode_label(locale: AppLocale, mode: TrayUsageDisplayMode) -> &'static str {
    match mode {
        TrayUsageDisplayMode::Used => match locale {
            AppLocale::ZhCn => "已用",
            AppLocale::EnUs => "Used",
            AppLocale::JaJp => "使用済み",
            AppLocale::KoKr => "사용",
            AppLocale::RuRu => "Использовано",
        },
        TrayUsageDisplayMode::Remaining => match locale {
            AppLocale::ZhCn => "剩余",
            AppLocale::EnUs => "Remaining",
            AppLocale::JaJp => "残り",
            AppLocale::KoKr => "남음",
            AppLocale::RuRu => "Осталось",
        },
        TrayUsageDisplayMode::Hidden => match locale {
            AppLocale::ZhCn => "不展示",
            AppLocale::EnUs => "Hidden",
            AppLocale::JaJp => "非表示",
            AppLocale::KoKr => "숨김",
            AppLocale::RuRu => "Скрыть",
        },
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_current_prefix(locale: AppLocale) -> String {
    match locale {
        AppLocale::ZhCn => "[当前] ",
        AppLocale::EnUs => "[Current] ",
        AppLocale::JaJp => "[現在] ",
        AppLocale::KoKr => "[현재] ",
        AppLocale::RuRu => "[Текущий] ",
    }
    .to_string()
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_usage_heading(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "CodexDeck 用量",
        AppLocale::EnUs => "CodexDeck Usage",
        AppLocale::JaJp => "CodexDeck 使用量",
        AppLocale::KoKr => "CodexDeck 사용량",
        AppLocale::RuRu => "Использование CodexDeck",
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_display_mode_label(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "状态栏展示",
        AppLocale::EnUs => "Status bar display",
        AppLocale::JaJp => "ステータスバー表示",
        AppLocale::KoKr => "상태바 표시",
        AppLocale::RuRu => "Отображение в строке меню",
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_current_label(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "当前",
        AppLocale::EnUs => "Current",
        AppLocale::JaJp => "現在",
        AppLocale::KoKr => "현재",
        AppLocale::RuRu => "Текущий",
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_current_account_label(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "当前账号",
        AppLocale::EnUs => "Current account",
        AppLocale::JaJp => "現在のアカウント",
        AppLocale::KoKr => "현재 계정",
        AppLocale::RuRu => "Текущий аккаунт",
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_no_current(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "未检测到正在使用的账号",
        AppLocale::EnUs => "No active account detected",
        AppLocale::JaJp => "使用中のアカウントが見つかりません",
        AppLocale::KoKr => "현재 사용 중인 계정을 찾을 수 없습니다",
        AppLocale::RuRu => "Активный аккаунт не обнаружен",
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_no_accounts(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "暂无账号，请先在主窗口添加账号",
        AppLocale::EnUs => "No accounts yet. Add one in the main window first",
        AppLocale::JaJp => "アカウントがありません。先にメイン画面で追加してください",
        AppLocale::KoKr => "계정이 없습니다. 먼저 메인 창에서 계정을 추가하세요",
        AppLocale::RuRu => "Аккаунтов пока нет. Сначала добавьте их в главном окне",
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_all_accounts(locale: AppLocale, count: usize) -> String {
    match locale {
        AppLocale::ZhCn => format!("全部账号（{count}）:"),
        AppLocale::EnUs => format!("All accounts ({count}):"),
        AppLocale::JaJp => format!("すべてのアカウント（{count}）:"),
        AppLocale::KoKr => format!("전체 계정({count}):"),
        AppLocale::RuRu => format!("Все аккаунты ({count}):"),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_more_accounts(locale: AppLocale, count: usize) -> String {
    match locale {
        AppLocale::ZhCn => format!("... 还有 {count} 个账号"),
        AppLocale::EnUs => format!("... and {count} more accounts"),
        AppLocale::JaJp => format!("... 他に {count} 件のアカウント"),
        AppLocale::KoKr => format!("... 계정 {count}개 더 있음"),
        AppLocale::RuRu => format!("... и еще {count} аккаунтов"),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_empty_accounts(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "暂无账号（请在主窗口添加）",
        AppLocale::EnUs => "No accounts (add one in the main window)",
        AppLocale::JaJp => "アカウントなし（メイン画面で追加してください）",
        AppLocale::KoKr => "계정 없음(메인 창에서 추가하세요)",
        AppLocale::RuRu => "Нет аккаунтов (добавьте в главном окне)",
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_refresh_now(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "立即刷新用量",
        AppLocale::EnUs => "Refresh usage now",
        AppLocale::JaJp => "今すぐ使用量を更新",
        AppLocale::KoKr => "지금 사용량 새로고침",
        AppLocale::RuRu => "Обновить использование",
    }
}

pub(crate) fn tray_open_app(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "打开 CodexDeck",
        AppLocale::EnUs => "Open CodexDeck",
        AppLocale::JaJp => "CodexDeck を開く",
        AppLocale::KoKr => "CodexDeck 열기",
        AppLocale::RuRu => "Открыть CodexDeck",
    }
}

pub(crate) fn tray_quit(locale: AppLocale) -> &'static str {
    match locale {
        AppLocale::ZhCn => "退出",
        AppLocale::EnUs => "Quit",
        AppLocale::JaJp => "終了",
        AppLocale::KoKr => "종료",
        AppLocale::RuRu => "Выход",
    }
}
