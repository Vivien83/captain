mod brand;
mod daemon;
mod environment;
mod local;
mod memory;

use crate::ui;

struct DoctorReport {
    checks: Vec<serde_json::Value>,
    all_ok: bool,
    repaired: bool,
    json: bool,
    repair: bool,
    full: bool,
}

impl DoctorReport {
    fn new(json: bool, repair: bool, full: bool) -> Self {
        Self {
            checks: Vec::new(),
            all_ok: true,
            repaired: false,
            json,
            repair,
            full,
        }
    }

    fn push(&mut self, value: serde_json::Value) {
        self.checks.push(value);
    }

    fn fail(&mut self) {
        self.all_ok = false;
    }

    fn mark_repaired(&mut self) {
        self.repaired = true;
    }

    fn success_hint(&self) -> Option<&'static str> {
        let daemon_running = self.checks.iter().any(|check| {
            check["check"].as_str() == Some("daemon") && check["status"].as_str() == Some("ok")
        });
        (!daemon_running).then_some("Start the daemon: captain start")
    }

    fn finish(self) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "all_ok": self.all_ok,
                    "checks": self.checks,
                }))
                .unwrap_or_default()
            );
            return;
        }

        println!();
        if self.all_ok {
            ui::success("All checks passed! Captain is ready.");
            if let Some(hint) = self.success_hint() {
                ui::hint(hint);
            }
        } else if self.repaired {
            ui::success("Repairs applied. Re-run `captain doctor` to verify.");
        } else {
            ui::error("Some checks failed.");
            if !self.repair {
                ui::hint("Run `captain doctor --repair` to attempt auto-fix");
            }
        }
    }
}

pub(crate) fn cmd_doctor(json: bool, repair: bool, full: bool, brand_audit: bool) {
    let mut report = DoctorReport::new(json, repair, full);

    if !report.json {
        ui::step("Captain Doctor");
        println!();
    }

    local::check_cli_install(&mut report);
    if dirs::home_dir().is_some() {
        local::check_home(&mut report);
    } else {
        if !report.json {
            ui::check_fail("Could not determine home directory");
        }
        report.push(serde_json::json!({"check": "home_dir", "status": "fail"}));
        report.fail();
    }

    environment::check_providers(&mut report);
    environment::check_channels(&mut report);
    environment::check_env_consistency(&mut report);
    memory::check_mempalace(&mut report);
    environment::check_config_deserialization(&mut report);
    environment::check_skills(&mut report);
    environment::check_extensions(&mut report);
    daemon::check_daemon_details(&mut report);
    daemon::check_runtime_tools(&mut report);

    if brand_audit {
        brand::check_brand_audit(&mut report);
    }

    report.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_running_daemon_does_not_get_start_hint() {
        let mut report = DoctorReport::new(false, false, false);
        report.push(serde_json::json!({"check": "daemon", "status": "ok"}));

        assert_eq!(report.success_hint(), None);
    }

    #[test]
    fn successful_offline_install_keeps_start_hint() {
        let mut report = DoctorReport::new(false, false, false);
        report.push(serde_json::json!({"check": "daemon", "status": "warn"}));

        assert_eq!(
            report.success_hint(),
            Some("Start the daemon: captain start")
        );
    }
}
