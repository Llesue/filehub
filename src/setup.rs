//! Windows first-run setup wizard.
//!
//! The wizard only runs on Windows. It collects the minimum configuration,
//! writes `config.json`, optionally installs the service from a stable copy of
//! the executable, waits for `/healthz`, then shows the address and the
//! one-time setup token.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{self, IsTerminal, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::platform;
use crate::service;

const DEFAULT_PORT: u16 = 8080;
const DEFAULT_LISTEN: &str = "0.0.0.0";
const DEFAULT_DATA_DIR: &str = r"C:\ProgramData\FileHub";
const INSTALL_DIR: &str = r"C:\Program Files\FileHub";
const TOKEN_FILE: &str = "setup-token.txt";
const FIREWALL_RULE: &str = "FileHub";

pub enum Outcome {
    /// Nothing left for the caller to start.
    Exit,
    /// Run the server in the foreground with this config file.
    Foreground(PathBuf),
}

#[derive(Debug, Serialize, Deserialize)]
struct ElevationRequest {
    config_path: PathBuf,
    data_dir: PathBuf,
    listen: String,
}

/// Run the wizard, or the elevated installation phase when `elevated` points
/// at a request file written by the unelevated wizard.
pub fn run_wizard(yes: bool, elevated: Option<&Path>) -> anyhow::Result<Outcome> {
    if let Some(request) = elevated {
        run_elevated(request)?;
        return Ok(Outcome::Exit);
    }
    if !yes && !io::stdin().is_terminal() {
        bail!("setup wizard needs an interactive console; use --yes for defaults");
    }

    println!("FileHub 首次启动向导");
    println!();

    if open_existing_if_running() {
        return Ok(Outcome::Exit);
    }

    let existing_config = platform::default_config_path();

    if existing_config.exists() {
        println!("检测到已有配置：{}", existing_config.display());
        if !yes {
            println!("  1) 使用现有配置启动（默认）");
            println!("  2) 重新配置（不会删除已有数据库）");
            let choice = prompt("选择 [1]: ")?;
            if choice != "2" {
                return use_existing(&existing_config);
            }
        } else {
            return use_existing(&existing_config);
        }
    }

    let default_data =
        existing_data_dir(&existing_config).unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR));
    let defaults = if yes {
        WizardAnswers {
            port: DEFAULT_PORT,
            address: DEFAULT_LISTEN.to_owned(),
            data_dir: default_data,
            install_service: true,
        }
    } else {
        ask_questions(default_data)?
    };

    let ip: IpAddr = defaults
        .address
        .parse()
        .with_context(|| format!("无效的监听地址：{}", defaults.address))?;
    let listen = SocketAddr::new(ip, defaults.port);
    if is_filehub(listen) {
        let url = local_url(listen);
        println!("端口 {} 上已经有 FileHub：{url}", defaults.port);
        open_browser(&url);
        return Ok(Outcome::Exit);
    }
    if !port_available(listen) {
        bail!(
            "端口 {} 已被其他程序占用，请换一个端口后重试",
            defaults.port
        );
    }

    if !defaults.install_service {
        std::fs::create_dir_all(&defaults.data_dir)
            .with_context(|| format!("无法创建数据目录 {}", defaults.data_dir.display()))?;
        std::fs::create_dir_all(defaults.data_dir.join("users")).context("无法创建用户目录")?;
        std::fs::create_dir_all(defaults.data_dir.join("cache")).context("无法创建缓存目录")?;
        let config_path = defaults.data_dir.join("config.json");
        write_config(&config_path, &defaults.data_dir, listen)?;
        println!();
        println!("配置已写入：{}", config_path.display());
        println!("前台模式：关闭这个窗口会停止 FileHub。");
        return Ok(Outcome::Foreground(config_path));
    }

    let config_path = defaults.data_dir.join("config.json");
    println!();
    println!("即将安装 Windows 服务，可能会请求管理员权限...");
    let request = ElevationRequest {
        config_path: config_path.clone(),
        data_dir: defaults.data_dir.clone(),
        listen: listen.to_string(),
    };
    let request_path =
        std::env::temp_dir().join(format!("filehub-setup-{}.json", std::process::id()));
    std::fs::write(&request_path, serde_json::to_vec_pretty(&request)?)
        .with_context(|| format!("无法写入临时配置 {}", request_path.display()))?;

    if is_elevated() {
        run_elevated(&request_path)?;
    } else {
        elevate(&request_path)?;
    }
    let _ = std::fs::remove_file(&request_path);
    Ok(Outcome::Exit)
}

struct WizardAnswers {
    port: u16,
    address: String,
    data_dir: PathBuf,
    install_service: bool,
}

fn ask_questions(default_data: PathBuf) -> anyhow::Result<WizardAnswers> {
    let port = ask_port(DEFAULT_PORT)?;
    let address = ask_address(DEFAULT_LISTEN)?;
    let data_dir = ask_data_dir(&default_data)?;
    let install_service = ask_yes_no("安装为 Windows 服务", true)?;
    Ok(WizardAnswers {
        port,
        address,
        data_dir,
        install_service,
    })
}

fn ask_port(default: u16) -> anyhow::Result<u16> {
    loop {
        let value = prompt(&format!("端口 [{default}]: "))?;
        if value.is_empty() {
            return Ok(default);
        }
        match value.parse::<u16>() {
            Ok(port) if port > 0 => return Ok(port),
            _ => println!("端口必须是 1-65535 的数字。"),
        }
    }
}

fn ask_address(default: &str) -> anyhow::Result<String> {
    loop {
        let value = prompt(&format!(
            "监听地址（0.0.0.0 允许局域网访问，127.0.0.1 仅本机）[{default}]: "
        ))?;
        let value = if value.is_empty() {
            default.to_owned()
        } else {
            value
        };
        match value.parse::<IpAddr>() {
            Ok(_) => return Ok(value),
            Err(_) => println!("请输入有效的 IP 地址，例如 0.0.0.0 或 127.0.0.1。"),
        }
    }
}

fn ask_data_dir(default: &Path) -> anyhow::Result<PathBuf> {
    loop {
        let value = prompt(&format!("数据目录 [{}]: ", default.display()))?;
        let path = if value.is_empty() {
            default.to_path_buf()
        } else {
            PathBuf::from(value)
        };
        if !path.is_absolute() {
            println!("请输入绝对路径。");
            continue;
        }
        return Ok(path);
    }
}

fn ask_yes_no(label: &str, default: bool) -> anyhow::Result<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        let value = prompt(&format!("{label} {suffix}: "))?;
        if value.is_empty() {
            return Ok(default);
        }
        match value.to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("请输入 y 或 n。"),
        }
    }
}

fn prompt(label: &str) -> anyhow::Result<String> {
    print!("{label}");
    io::stdout().flush().context("无法刷新提示符")?;
    let mut line = String::new();
    io::stdin().read_line(&mut line).context("无法读取输入")?;
    Ok(line.trim().to_owned())
}

fn use_existing(config_path: &Path) -> anyhow::Result<Outcome> {
    if service::query()?.is_some() {
        service::start_existing().context("无法启动已有 FileHub 服务")?;
        let listen = existing_listen(config_path).unwrap_or(default_listen());
        let url = local_url(listen);
        println!("FileHub 服务已启动：{url}");
        open_browser(&url);
        return Ok(Outcome::Exit);
    }
    Ok(Outcome::Foreground(config_path.to_path_buf()))
}

fn run_elevated(request_path: &Path) -> anyhow::Result<()> {
    let raw = std::fs::read(request_path)
        .with_context(|| format!("无法读取安装请求 {}", request_path.display()))?;
    let request: ElevationRequest = serde_json::from_slice(&raw).context("安装请求格式无效")?;
    let listen: SocketAddr = request.listen.parse().context("监听地址无效")?;

    std::fs::create_dir_all(&request.data_dir)
        .with_context(|| format!("无法创建数据目录 {}", request.data_dir.display()))?;
    std::fs::create_dir_all(request.data_dir.join("users")).context("无法创建用户目录")?;
    std::fs::create_dir_all(request.data_dir.join("cache")).context("无法创建缓存目录")?;
    write_config(&request.config_path, &request.data_dir, listen)?;

    let executable = install_executable()?;
    service::install_at(&executable, Some(&request.config_path), true)
        .context("安装 FileHub 服务失败")?;
    if listen.ip().is_unspecified() {
        add_firewall_rule(listen.port());
    }

    wait_for_health(listen)?;
    let url = local_url(listen);
    println!();
    println!("FileHub 已启动");
    println!("地址：{url}");

    let token_path = request.data_dir.join(TOKEN_FILE);
    match read_setup_token(&token_path) {
        Some(token) => {
            if copy_to_clipboard(&token) {
                println!("setup token 已复制到剪贴板");
            } else {
                println!("setup token：{token}");
            }
        }
        None => {
            println!("没有待完成的首次设置（已完成初始化，或 token 文件不可读）");
        }
    }

    open_browser(&url);
    println!();
    println!("30 秒后自动关闭，按回车立即关闭...");
    countdown(Duration::from_secs(30));
    Ok(())
}

fn install_executable() -> anyhow::Result<PathBuf> {
    let current = std::env::current_exe().context("无法定位 FileHub 可执行文件")?;
    let target = Path::new(INSTALL_DIR).join("filehub.exe");
    std::fs::create_dir_all(INSTALL_DIR)
        .with_context(|| format!("无法创建安装目录 {INSTALL_DIR}"))?;
    if !platform::paths_equal(&current, &target) {
        std::fs::copy(&current, &target)
            .with_context(|| format!("无法复制 {} 到 {}", current.display(), target.display()))?;
    }
    Ok(target)
}

fn write_config(config_path: &Path, data_dir: &Path, listen: SocketAddr) -> anyhow::Result<()> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建配置目录 {}", parent.display()))?;
    }
    let value = serde_json::json!({
        "listen": listen.to_string(),
        "db_path": data_dir.join("filehub.db").to_string_lossy(),
        "users_root": data_dir.join("users").to_string_lossy(),
        "cache_dir": data_dir.join("cache").to_string_lossy(),
    });
    std::fs::write(config_path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("无法写入配置 {}", config_path.display()))?;
    Ok(())
}

fn read_setup_token(path: &Path) -> Option<String> {
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn copy_to_clipboard(text: &str) -> bool {
    let Ok(mut child) = Command::new("clip").stdin(Stdio::piped()).spawn() else {
        return false;
    };
    if let Some(stdin) = child.stdin.as_mut()
        && stdin.write_all(text.as_bytes()).is_err()
    {
        return false;
    }
    child.wait().map(|status| status.success()).unwrap_or(false)
}

fn countdown(total: Duration) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
        let _ = tx.send(());
    });

    let seconds = total.as_secs();
    for remaining in (1..=seconds).rev() {
        print!("\r{remaining} 秒后自动关闭，按回车立即关闭... ");
        let _ = io::stdout().flush();
        if rx.recv_timeout(Duration::from_secs(1)).is_ok() {
            break;
        }
    }
    println!();
}

fn elevate(request_path: &Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("无法定位 FileHub 可执行文件")?;
    let quoted_request = format!("\"{}\"", request_path.display());
    let script = format!(
        "Start-Process -FilePath '{}' -ArgumentList @('setup','--elevated','{}') -Verb RunAs -Wait",
        ps_escape(&exe.to_string_lossy()),
        ps_escape(&quoted_request),
    );
    let mut utf16 = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .status()
        .context("无法启动管理员权限请求")?;
    if !status.success() {
        bail!("管理员权限请求被取消，服务未安装");
    }
    Ok(())
}

fn ps_escape(value: &str) -> String {
    value.replace('\'', "''")
}

fn is_elevated() -> bool {
    Command::new("net")
        .arg("session")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn service_is_running() -> bool {
    matches!(
        service::query(),
        Ok(Some(windows_service::service::ServiceState::Running))
    )
}

/// If the installed service or another FileHub instance already serves the
/// configured address, open it and return true.
pub fn open_existing_if_running() -> bool {
    let config_path = platform::default_config_path();
    let listen = existing_listen(&config_path).unwrap_or(default_listen());
    if service_is_running() || is_filehub(listen) {
        let url = local_url(listen);
        println!("FileHub 已经在运行：{url}");
        open_browser(&url);
        return true;
    }
    false
}

fn existing_listen(config_path: &Path) -> Option<SocketAddr> {
    let config = crate::config::load_config_file(Some(config_path)).ok()?;
    let snapshot = crate::config::build_snapshot(&config, &[]).ok()?;
    Some(snapshot.settings.listen)
}

fn existing_data_dir(config_path: &Path) -> Option<PathBuf> {
    let config = crate::config::load_config_file(Some(config_path)).ok()?;
    let snapshot = crate::config::build_snapshot(&config, &[]).ok()?;
    snapshot.settings.db_path.parent().map(Path::to_path_buf)
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT))
}

fn local_url(listen: SocketAddr) -> String {
    let host = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_owned(),
        IpAddr::V6(ip) if ip.is_unspecified() => "[::1]".to_owned(),
        IpAddr::V6(ip) => format!("[{ip}]"),
        IpAddr::V4(ip) => ip.to_string(),
    };
    format!("http://{host}:{}/", listen.port())
}

fn port_available(listen: SocketAddr) -> bool {
    TcpListener::bind(listen).is_ok()
}

fn is_filehub(listen: SocketAddr) -> bool {
    http_get(listen, "/healthz")
        .is_some_and(|response| response.contains(" 200") && response.contains("filehub"))
}

fn wait_for_health(listen: SocketAddr) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if is_filehub(listen) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("FileHub 没有在 45 秒内就绪，请查看 {INSTALL_DIR}\\filehub.log.* 或服务事件")
}

fn probe_address(listen: SocketAddr) -> SocketAddr {
    if !listen.ip().is_unspecified() {
        return listen;
    }
    match listen.ip() {
        IpAddr::V4(_) => SocketAddr::from(([127, 0, 0, 1], listen.port())),
        IpAddr::V6(_) => SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], listen.port())),
    }
}

fn http_get(listen: SocketAddr, path: &str) -> Option<String> {
    let address = probe_address(listen);
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

fn open_browser(url: &str) {
    let _ = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .status();
}

fn add_firewall_rule(port: u16) {
    let rule = format!("name={FIREWALL_RULE}");
    let local_port = format!("localport={port}");
    let _ = Command::new("netsh")
        .args(["advfirewall", "firewall", "delete", "rule", rule.as_str()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            rule.as_str(),
            "dir=in",
            "action=allow",
            "protocol=TCP",
            local_port.as_str(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Keep the window open after a failed wizard run so the user can read it.
pub fn pause() {
    if io::stdin().is_terminal() {
        eprintln!();
        eprintln!("按回车退出...");
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
    }
}
