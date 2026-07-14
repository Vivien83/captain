//! REST surface for v3.11 projects — routes called by the Next.js UI
//! and external tooling. Each handler is a thin wrapper over the
//! MemorySubstrate helpers; error → 400/404/500 with a JSON body.

pub use crate::project_active_routes::{
    clear_active_project, get_active_project, set_active_project, SetActiveProjectReq,
};
pub use crate::project_archive_routes::archive_project;
pub use crate::project_checkpoint_routes::{
    create_checkpoint, list_checkpoints, CreateCheckpointReq,
};
pub use crate::project_create_routes::{create_project, CreateProjectReq};
pub use crate::project_delete_routes::delete_project;
pub use crate::project_detail_routes::get_project_by_slug;
pub use crate::project_environment_routes::projects_environment;
pub use crate::project_goal_routes::{
    create_project_goal, delete_project_goal, list_project_goals, pause_project_goal,
    resume_project_goal, update_project_goal, CreateProjectGoalReq, UpdateProjectGoalReq,
};
pub use crate::project_launch_routes::launch_project;
pub use crate::project_lifecycle_routes::{set_project_lifecycle_phase, SetLifecyclePhaseReq};
pub use crate::project_list_routes::list_projects;
pub use crate::project_milestone_routes::{
    complete_milestone, create_milestone, get_milestone_progress, list_milestones,
    CreateMilestoneReq,
};
pub use crate::project_resume_routes::resume_project;
pub use crate::project_runtime_pause_routes::pause_project_runtime;
pub use crate::project_runtime_resume_routes::resume_project_runtime;
pub use crate::project_runtime_routes::{project_runtime, ProjectRuntimeQuery};
pub use crate::project_runtime_start_routes::start_project_runtime;
pub use crate::project_runtime_takeover_routes::takeover_project_runtime;
pub use crate::project_task_routes::{
    create_project_task, delete_project_task, list_project_tasks, update_project_task,
    CreateTaskReq, UpdateTaskReq,
};
pub use crate::project_update_routes::{update_project, UpdateProjectReq};
