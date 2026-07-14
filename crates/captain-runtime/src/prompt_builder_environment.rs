pub(super) fn build_language_contract_section(configured_language: Option<&str>) -> Option<String> {
    let lang = configured_language?.trim();
    if lang.is_empty() {
        return None;
    }
    let lower = lang.to_ascii_lowercase();
    let label = match lower.as_str() {
        "fr" | "fr-fr" | "fr_fr" | "french" | "français" | "francais" => "French / français",
        "en" | "en-us" | "en_us" | "en-gb" | "en_gb" | "english" => "English",
        _ => lang,
    };
    Some(format!(
        "## Language Contract\nConfigured user language: {label}. \
         Answer in that language from the first turn unless the latest user message clearly asks for another language."
    ))
}

pub(super) fn build_deployment_context_section(deployment_profile: Option<&str>) -> Option<String> {
    let profile = deployment_profile?.trim().to_ascii_lowercase();
    if profile != "vps" {
        return None;
    }
    Some(
        "## Deployment Context\n\
         Captain is installed directly on the user's VPS. Treat this host as Captain's local execution environment. \
         For questions about this VPS health, OS, disk, RAM, processes, Docker, ports, services, logs, or uptime, \
         prefer local `shell_exec` first because Captain is already running on that machine. \
         Use `ssh_exec` only when the user asks about another host, a stored SSH alias, or an explicitly remote target."
            .to_string(),
    )
}

/// Phase P.1: build the runtime environment section. Always present in
/// the system prompt so the LLM picks the right shell command on the
/// first try (vm_stat on macOS, free on Linux, tasklist on Windows...)
/// instead of erroring out and retrying.
pub(super) fn build_environment_section() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into());
    let home = dirs::home_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let cheatsheet = match os {
        "macos" => MACOS_CHEATSHEET,
        "linux" => LINUX_CHEATSHEET,
        "windows" => WINDOWS_CHEATSHEET,
        _ => "",
    };

    let mut s = String::with_capacity(512);
    s.push_str("## Environment\n");
    s.push_str(&format!("- OS: {os} ({arch})\n"));
    s.push_str(&format!("- Shell: {shell}\n"));
    if !home.is_empty() {
        s.push_str(&format!("- User home: {home}\n"));
    }
    if !cwd.is_empty() {
        s.push_str(&format!("- Working directory: {cwd}\n"));
    }
    if !cheatsheet.is_empty() {
        s.push('\n');
        s.push_str(cheatsheet);
    }
    s
}

const MACOS_CHEATSHEET: &str = "Use macOS-native shell commands (NOT Linux equivalents):\n\
- RAM/memory: `vm_stat` (NOT `free`)\n\
- Processes: `ps -A` or `top -l 1 -n 0`\n\
- Disk: `df -h` (same)\n\
- Network connections: `lsof -iTCP -sTCP:LISTEN -nP` or `netstat -an`\n\
- Open file/URL: `open <path>`\n\
- Clipboard: `pbcopy` (write) / `pbpaste` (read)\n\
- Package manager: `brew install <pkg>`\n\
- Find files: `mdfind <name>` (Spotlight) or `find . -name '*.x'`\n\
- System info: `sw_vers`, `system_profiler SPSoftwareDataType`\n\
- Date: `date` (same)\n\
- DO NOT use `free`, `xdg-open`, `xclip`, `apt`, `yum`, `tasklist`.\n\
- If a command isn't macOS-native, look up the equivalent BEFORE running.";

const LINUX_CHEATSHEET: &str = "Use Linux-native shell commands:\n\
- RAM/memory: `free -h`\n\
- Processes: `ps aux` or `top -bn1`\n\
- Disk: `df -h`\n\
- Network connections: `ss -tlnp` or `netstat -tlnp`\n\
- Open file/URL: `xdg-open <path>`\n\
- Clipboard: `xclip -selection clipboard` or `xsel --clipboard`\n\
- Package manager: `apt`, `dnf`, `pacman` (depends on distro — check with `cat /etc/os-release`)\n\
- Find files: `find . -name '*.x'`\n\
- System info: `uname -a`, `lsb_release -a`\n\
- DO NOT use `vm_stat`, `pbcopy`, `open`, `tasklist`.";

const WINDOWS_CHEATSHEET: &str = "Use Windows-native commands (PowerShell preferred):\n\
- RAM/memory: `Get-CimInstance Win32_OperatingSystem | Select FreePhysicalMemory,TotalVisibleMemorySize`\n\
- Processes: `tasklist` or `Get-Process`\n\
- Disk: `Get-PSDrive` or `wmic logicaldisk get size,freespace,caption`\n\
- Network connections: `netstat -ano` or `Get-NetTCPConnection`\n\
- Open file/URL: `start <path>`\n\
- Clipboard: `clip` (write) / `Get-Clipboard` (read)\n\
- Find files: `dir /s /b <name>` or `Get-ChildItem -Recurse -Filter <name>`\n\
- DO NOT use `vm_stat`, `free`, `pbcopy`, `xdg-open`, `apt`.";
