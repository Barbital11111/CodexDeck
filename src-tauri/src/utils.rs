use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub(crate) fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn short_account(account_id: &str) -> String {
    account_id.chars().take(8).collect()
}

pub(crate) fn truncate_for_error(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        value.to_string()
    } else {
        let truncated = value.chars().take(max_len).collect::<String>();
        format!("{truncated}...")
    }
}

pub(crate) fn redact_sensitive_text(value: &str) -> String {
    let mut redacted = redact_token_like(value);
    redacted = redact_email_like(&redacted);
    redacted = redact_url_like(&redacted);
    redact_local_paths(&redacted)
}

fn redact_token_like(value: &str) -> String {
    let mut redacted = value.to_string();
    for prefix in ["sk-", "tp-"] {
        redacted = redact_prefixed_token(&redacted, prefix);
    }
    redacted = redact_named_token_fields(&redacted);
    redacted = redact_bearer_tokens(&redacted);
    redact_jwt_like(&redacted)
}

fn redact_named_token_fields(value: &str) -> String {
    let mut redacted = value.to_string();
    for field_name in [
        "access_token",
        "refresh_token",
        "id_token",
        "accessToken",
        "refreshToken",
        "idToken",
    ] {
        redacted = redact_named_token_field(&redacted, field_name);
    }
    redacted
}

fn redact_named_token_field(value: &str, field_name: &str) -> String {
    let lower_value = value.to_ascii_lowercase();
    let lower_field_name = field_name.to_ascii_lowercase();
    let mut output = String::with_capacity(value.len());
    let mut index = 0usize;

    while let Some(relative_start) = lower_value[index..].find(&lower_field_name) {
        let start = index + relative_start;
        if !is_named_token_field_start(value, start) {
            output.push_str(&value[index..start + 1]);
            index = start + 1;
            continue;
        }

        let Some((token_start, token_end)) =
            named_token_value_bounds(value, start + field_name.len())
        else {
            output.push_str(&value[index..start + field_name.len()]);
            index = start + field_name.len();
            continue;
        };

        if token_start == token_end {
            output.push_str(&value[index..token_end]);
            index = token_end;
            continue;
        }

        output.push_str(&value[index..token_start]);
        output.push_str("[已隐藏令牌]");
        index = token_end;
    }

    output.push_str(&value[index..]);
    output
}

fn is_named_token_field_start(value: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    let previous = value[..start].chars().next_back();
    previous.is_none_or(|ch| !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-'))
}

fn named_token_value_bounds(value: &str, field_end: usize) -> Option<(usize, usize)> {
    let mut cursor = field_end;

    if let Some((quote, next_cursor)) = consume_optional_quote(value, cursor) {
        cursor = next_cursor;
        cursor = skip_ascii_whitespace(value, cursor);
        if !matches!(value[cursor..].chars().next(), Some(':') | Some('=')) {
            return None;
        }
        cursor += 1;
        cursor = skip_ascii_whitespace(value, cursor);
        return quoted_or_bare_token_bounds(value, cursor, Some(quote));
    }

    cursor = skip_ascii_whitespace(value, cursor);
    if !matches!(value[cursor..].chars().next(), Some(':') | Some('=')) {
        return None;
    }
    cursor += 1;
    cursor = skip_ascii_whitespace(value, cursor);
    quoted_or_bare_token_bounds(value, cursor, None)
}

fn consume_optional_quote(value: &str, cursor: usize) -> Option<(char, usize)> {
    match value[cursor..].chars().next() {
        Some(quote @ ('"' | '\'')) => Some((quote, cursor + quote.len_utf8())),
        _ => None,
    }
}

fn skip_ascii_whitespace(value: &str, mut cursor: usize) -> usize {
    while matches!(value[cursor..].chars().next(), Some(ch) if ch.is_ascii_whitespace()) {
        cursor += value[cursor..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(0);
    }
    cursor
}

fn quoted_or_bare_token_bounds(
    value: &str,
    cursor: usize,
    field_quote: Option<char>,
) -> Option<(usize, usize)> {
    let value_quote = match value[cursor..].chars().next() {
        Some(quote @ ('"' | '\'')) => Some(quote),
        _ => field_quote.filter(|_| false),
    };
    let token_start = value_quote
        .map(|quote| cursor + quote.len_utf8())
        .unwrap_or(cursor);
    let token_end = find_named_token_value_end(value, token_start, value_quote);
    Some((token_start, token_end))
}

fn find_named_token_value_end(value: &str, start: usize, quote: Option<char>) -> usize {
    for (relative_index, ch) in value[start..].char_indices() {
        if quote.is_some_and(|quote| ch == quote)
            || quote.is_none()
                && (ch.is_whitespace() || matches!(ch, ',' | ';' | '&' | ')' | ']' | '}'))
        {
            return start + relative_index;
        }
    }
    value.len()
}

fn redact_prefixed_token(value: &str, prefix: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0usize;
    while let Some(relative_start) = value[index..].find(prefix) {
        let start = index + relative_start;
        output.push_str(&value[index..start]);
        let end = value[start..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .map(|offset| start + offset)
            .unwrap_or(value.len());
        if end.saturating_sub(start) >= 12 {
            output.push_str("[已隐藏密钥]");
        } else {
            output.push_str(&value[start..end]);
        }
        index = end;
    }
    output.push_str(&value[index..]);
    output
}

fn redact_bearer_tokens(value: &str) -> String {
    let mut output = Vec::new();
    let mut hide_next = false;
    for part in value.split_whitespace() {
        if hide_next {
            output.push("[已隐藏密钥]".to_string());
            hide_next = false;
            continue;
        }
        if part.eq_ignore_ascii_case("bearer") {
            output.push(part.to_string());
            hide_next = true;
        } else {
            output.push(part.to_string());
        }
    }
    output.join(" ")
}

fn redact_jwt_like(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| {
            let trimmed = part.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | ',' | ';' | ')' | ']' | '}' | '(' | '[' | '{'
                )
            });
            let token_start = trimmed.find("eyJ").unwrap_or(0);
            let candidate = &trimmed[token_start..];
            let dot_count = candidate.matches('.').count();
            if dot_count == 2 && candidate.len() >= 32 && candidate.starts_with("eyJ") {
                part.replace(candidate, "[已隐藏令牌]")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_email_like(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| {
            let trimmed = part
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';' | ')' | ']' | '}'));
            if trimmed.contains('@') && trimmed.contains('.') {
                part.replace(trimmed, "[已隐藏邮箱]")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_url_like(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0usize;
    while let Some(relative_start) = value[index..].find("http") {
        let start = index + relative_start;
        let scheme = if value[start..].starts_with("https://") {
            "https://"
        } else if value[start..].starts_with("http://") {
            "http://"
        } else {
            output.push_str(&value[index..=start]);
            index = start + 1;
            continue;
        };
        output.push_str(&value[index..start]);
        let end = value[start..]
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | ')' | ']'))
            .map(|offset| start + offset)
            .unwrap_or(value.len());
        let url = &value[start..end];
        if is_local_url(url) {
            output.push_str(url);
        } else {
            output.push_str(scheme);
            output.push_str("[已隐藏地址]");
        }
        index = end;
    }
    output.push_str(&value[index..]);
    output
}

fn is_local_url(value: &str) -> bool {
    value.starts_with("http://127.0.0.1")
        || value.starts_with("http://localhost")
        || value.starts_with("https://localhost")
        || value.starts_with("http://0.0.0.0")
}

fn redact_local_paths(value: &str) -> String {
    let current_user = env::var("USERNAME")
        .or_else(|_| env::var("USER"))
        .unwrap_or_default();
    let mut redacted = value.to_string();
    redacted = redact_windows_absolute_paths(&redacted);
    redacted = redact_unc_paths(&redacted);
    for marker in ["C:\\Users\\", "C:/Users/", "/Users/", "/home/"] {
        while let Some(start) = redacted.find(marker) {
            let end = redacted[start..]
                .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';'))
                .map(|offset| start + offset)
                .unwrap_or(redacted.len());
            redacted.replace_range(start..end, "[已隐藏本地路径]");
        }
    }
    if !current_user.is_empty() {
        redacted = redacted.replace(&current_user, "[已隐藏用户]");
    }
    redacted
}

fn redact_windows_absolute_paths(value: &str) -> String {
    let chars = value.char_indices().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut last = 0usize;
    let mut index = 0usize;
    while index + 2 < chars.len() {
        let (_, drive) = chars[index];
        let (_, colon) = chars[index + 1];
        let (_, slash) = chars[index + 2];
        if drive.is_ascii_alphabetic()
            && colon == ':'
            && matches!(slash, '\\' | '/')
            && is_windows_drive_path_boundary(value, chars[index].0)
        {
            let start = chars[index].0;
            let end = find_path_end(value, start);
            output.push_str(&value[last..start]);
            output.push_str("[已隐藏本地路径]");
            last = end;
            while index < chars.len() && chars[index].0 < end {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    output.push_str(&value[last..]);
    output
}

fn redact_unc_paths(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut last = 0usize;
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' && bytes[index + 1] == b'\\' {
            let start = index;
            let end = find_path_end(value, start);
            output.push_str(&value[last..start]);
            output.push_str("[已隐藏本地路径]");
            last = end;
            index = end;
        } else {
            index += 1;
        }
    }
    output.push_str(&value[last..]);
    output
}

fn is_windows_drive_path_boundary(value: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    let previous = value[..start].chars().next_back();
    previous.is_none_or(|ch| {
        ch.is_whitespace() || matches!(ch, '"' | '\'' | '(' | '[' | '{' | ',' | ';')
    })
}

fn find_path_end(value: &str, start: usize) -> usize {
    value[start..]
        .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';'))
        .map(|offset| start + offset)
        .unwrap_or(value.len())
}

pub(crate) fn set_private_permissions(path: &Path) {
    let _ = try_set_private_permissions(path);
}

pub(crate) fn try_set_private_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .map_err(|error| format!("读取文件权限失败 {}: {error}", path.display()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("设置文件权限失败 {}: {error}", path.display()))?;
        Ok(())
    }

    #[cfg(windows)]
    {
        tighten_windows_private_file_acl(path)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(windows)]
fn tighten_windows_private_file_acl(path: &Path) -> Result<(), String> {
    let escaped_path = path.to_string_lossy().replace('\'', "''");
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$Path = '{escaped_path}'
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$acl = Get-Acl -LiteralPath $Path
$acl.SetAccessRuleProtection($true, $false)
foreach ($rule in @($acl.Access)) {{
    [void]$acl.RemoveAccessRuleAll($rule)
}}
$accessRule = New-Object System.Security.AccessControl.FileSystemAccessRule(
    $identity.User,
    [System.Security.AccessControl.FileSystemRights]::FullControl,
    [System.Security.AccessControl.AccessControlType]::Allow
)
$acl.AddAccessRule($accessRule)
Set-Acl -LiteralPath $Path -AclObject $acl
"#
    );

    let output = new_resolved_command("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .output()
        .map_err(|error| {
            format!(
                "调用 PowerShell 设置私有文件权限失败 {}: {error}",
                path.display()
            )
        })?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("退出码 {:?}", output.status.code())
        };
        Err(format!(
            "设置 Windows 私有文件 ACL 失败 {}: {detail}",
            path.display()
        ))
    }
}

pub(crate) fn prepare_process_path() {
    let mut merged = preferred_executable_dirs();
    if let Some(current_path) = env::var_os("PATH") {
        for dir in env::split_paths(&current_path) {
            push_unique_dir(&mut merged, dir);
        }
    }

    if let Ok(path_env) = env::join_paths(merged) {
        env::set_var("PATH", path_env);
    }
}

pub(crate) fn find_command_path(command: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path_os) = env::var_os("PATH") {
        for dir in env::split_paths(&path_os) {
            push_command_candidates_from_dir(&mut candidates, &dir, command);
        }
    }

    for dir in preferred_executable_dirs() {
        push_command_candidates_from_dir(&mut candidates, &dir, command);
    }

    candidates.into_iter().find(|path| is_executable_file(path))
}

pub(crate) fn configure_background_command(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

pub(crate) fn new_background_command<S: AsRef<OsStr>>(program: S) -> Command {
    let mut command = Command::new(program);
    configure_background_command(&mut command);
    command
}

pub(crate) fn new_resolved_command(command: &str) -> Command {
    let program = find_command_path(command).unwrap_or_else(|| PathBuf::from(command));
    let mut command = new_background_command(&program);
    if let Some(parent) = program.parent().filter(|_| program.is_absolute()) {
        if let Some(path_env) = prepend_path_entry(parent) {
            command.env("PATH", path_env);
        }
    }
    command
}

pub(crate) fn prepend_path_entry(path: &Path) -> Option<OsString> {
    let mut paths = vec![path.to_path_buf()];
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }
    env::join_paths(paths).ok()
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn preferred_executable_dirs() -> Vec<PathBuf> {
    preferred_executable_dir_candidates()
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect()
}

fn preferred_executable_dir_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        for dir in [
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/opt/homebrew/sbin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/local/sbin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/Library/Apple/usr/bin"),
        ] {
            push_unique_dir(&mut dirs, dir);
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for dir in [
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/local/sbin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/snap/bin"),
            PathBuf::from("/var/lib/flatpak/exports/bin"),
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
            PathBuf::from("/home/linuxbrew/.linuxbrew/sbin"),
            PathBuf::from("/nix/var/nix/profiles/default/bin"),
            PathBuf::from("/run/current-system/sw/bin"),
        ] {
            push_unique_candidate(&mut dirs, dir);
        }
    }

    #[cfg(target_os = "windows")]
    {
        for dir in [PathBuf::from(
            r"C:\Program Files\Docker\Docker\resources\bin",
        )] {
            push_unique_candidate(&mut dirs, dir);
        }
    }

    if let Some(cargo_home) = env::var_os("CARGO_HOME").map(PathBuf::from) {
        push_unique_candidate(&mut dirs, cargo_home.join("bin"));
    }

    if let Some(homebrew_prefix) = env::var_os("HOMEBREW_PREFIX").map(PathBuf::from) {
        push_unique_candidate(&mut dirs, homebrew_prefix.join("bin"));
        push_unique_candidate(&mut dirs, homebrew_prefix.join("sbin"));
    }

    if let Some(pnpm_home) = env::var_os("PNPM_HOME").map(PathBuf::from) {
        push_unique_candidate(&mut dirs, pnpm_home);
    }

    if let Some(home) = dirs::home_dir() {
        for dir in [
            home.join(".cargo").join("bin"),
            home.join(".local").join("bin"),
            home.join("bin"),
            home.join(".asdf").join("shims"),
            home.join(".volta").join("bin"),
            home.join(".npm-global").join("bin"),
            home.join(".linuxbrew").join("bin"),
            home.join(".linuxbrew").join("sbin"),
            home.join(".nix-profile").join("bin"),
            home.join(".rye").join("shims"),
            home.join(".local").join("share").join("mise").join("shims"),
            home.join("Library").join("pnpm"),
            home.join("scoop").join("shims"),
            home.join("AppData")
                .join("Local")
                .join("Microsoft")
                .join("WinGet")
                .join("Links"),
        ] {
            push_unique_candidate(&mut dirs, dir);
        }
    }

    dirs
}

fn push_unique_dir(dirs: &mut Vec<PathBuf>, candidate: PathBuf) {
    if candidate.is_dir() && !dirs.iter().any(|existing| existing == &candidate) {
        dirs.push(candidate);
    }
}

fn push_unique_candidate(dirs: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !dirs.iter().any(|existing| existing == &candidate) {
        dirs.push(candidate);
    }
}

fn push_command_candidates_from_dir(candidates: &mut Vec<PathBuf>, dir: &Path, command: &str) {
    #[cfg(windows)]
    {
        for name in [
            format!("{command}.exe"),
            format!("{command}.cmd"),
            format!("{command}.bat"),
        ] {
            candidates.push(dir.join(name));
        }
    }

    #[cfg(not(windows))]
    {
        candidates.push(dir.join(command));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let unique = format!(
            "codex-tools-utils-test-{name}-{}-{}",
            std::process::id(),
            now_unix_seconds()
        );
        env::temp_dir().join(unique)
    }

    fn restore_env_var(name: &str, original: Option<OsString>) {
        if let Some(value) = original {
            env::set_var(name, value);
        } else {
            env::remove_var(name);
        }
    }

    #[cfg(windows)]
    fn write_test_command(dir: &Path, command: &str) -> PathBuf {
        let path = dir.join(format!("{command}.cmd"));
        fs::write(&path, "@echo off\r\necho ok\r\n").expect("write test command");
        path
    }

    #[cfg(unix)]
    fn write_test_command(dir: &Path, command: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join(command);
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write test command");
        let mut permissions = fs::metadata(&path).expect("read metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("set execute bit");
        path
    }

    #[test]
    fn find_command_path_uses_cargo_home_when_path_is_missing() {
        let _guard = env_lock().lock().expect("lock env");
        let sandbox = unique_test_dir("cargo-home");
        let cargo_home = sandbox.join("cargo-home");
        let bin_dir = cargo_home.join("bin");
        let command_name = "codex-tools-test-probe";
        fs::create_dir_all(&bin_dir).expect("create cargo bin dir");
        let cargo_path = write_test_command(&bin_dir, command_name);

        let original_path = env::var_os("PATH");
        let original_cargo_home = env::var_os("CARGO_HOME");

        env::set_var("PATH", "");
        env::set_var("CARGO_HOME", &cargo_home);

        let resolved = find_command_path(command_name);

        restore_env_var("PATH", original_path);
        restore_env_var("CARGO_HOME", original_cargo_home);
        let _ = fs::remove_dir_all(&sandbox);

        assert_eq!(resolved, Some(cargo_path));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn preferred_candidates_include_common_unix_system_dirs() {
        let dirs = preferred_executable_dir_candidates();
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        assert!(dirs.contains(&PathBuf::from("/usr/bin")));
        assert!(dirs.contains(&PathBuf::from("/snap/bin")));
    }

    #[test]
    fn redact_sensitive_text_hides_keys_emails_and_local_paths() {
        let jwt = format!(
            "{}{}{}{}",
            "eyJhbGciOiJIUzI1NiJ9", ".", "eyJzdWIiOiIxMjMifQ", ".signature"
        );
        let fake_key = ["sk", "secret-token-123456"].join("-");
        let bearer = ["abcdefgh", "ijklmnop"].join("");
        let email = ["user", "example.invalid"].join("@");
        let local_path = ["C:", "Users", "alice", ".codex"].join("\\");
        let upstream = ["https://api", "example.invalid/v1"].join(".");
        let input = format!(
            "api_key={fake_key} Authorization: Bearer {bearer} jwt={jwt} {email} {local_path} {upstream} http://127.0.0.1:8787/v1"
        );
        let output = redact_sensitive_text(&input);

        assert!(!output.contains(&fake_key));
        assert!(!output.contains(&bearer));
        assert!(!output.contains("eyJhbGci"));
        assert!(!output.contains(&email));
        assert!(!output.contains(&local_path));
        assert!(!output.contains(&upstream));
        assert!(output.contains("http://127.0.0.1:8787/v1"));
        assert!(output.contains("[已隐藏密钥]"));
        assert!(output.contains("[已隐藏令牌]"));
        assert!(output.contains("[已隐藏邮箱]"));
        assert!(output.contains("[已隐藏地址]"));
        assert!(output.contains("[已隐藏本地路径]"));
    }

    #[test]
    fn redact_sensitive_text_hides_non_profile_absolute_paths() {
        let drive_path = ["D:", "\\AI\\workspace\\secret"].concat();
        let unc_path = ["\\\\", "server\\share\\secret"].concat();
        let input = format!("build manifest {drive_path} and {unc_path}");

        let output = redact_sensitive_text(&input);

        assert!(!output.contains(&drive_path));
        assert!(!output.contains(&unc_path));
        assert!(output.contains("[已隐藏本地路径]"));
    }

    #[test]
    fn redact_sensitive_text_hides_named_token_fields() {
        let input = concat!(
            r#"{"access_token":"atk","refresh_token":"rtk"}"#,
            " id_token=idk&next=ok accessToken: 'catk'"
        );

        let output = redact_sensitive_text(input);

        assert!(!output.contains("\"atk\""));
        assert!(!output.contains("\"rtk\""));
        assert!(!output.contains("idk"));
        assert!(!output.contains("'catk'"));
        assert!(output.contains("[已隐藏令牌]"));
        assert!(output.contains("next=ok"));
    }

    #[test]
    fn truncate_for_error_handles_multibyte_text() {
        let value = "中文🙂错误详情";
        let output = truncate_for_error(value, 4);

        assert_eq!(output, "中文🙂错...");
    }
}
