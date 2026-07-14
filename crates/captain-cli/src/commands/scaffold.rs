use crate::{ui, ScaffoldKind};

pub(crate) fn cmd_scaffold(kind: ScaffoldKind) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let result = match kind {
        ScaffoldKind::Skill => captain_extensions::installer::scaffold_skill(&cwd.join("my-skill")),
        ScaffoldKind::Integration => {
            captain_extensions::installer::scaffold_integration(&cwd.join("my-integration"))
        }
    };
    match result {
        Ok(msg) => ui::success(&msg),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}
