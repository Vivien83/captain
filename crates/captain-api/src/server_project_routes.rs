use crate::routes::AppState;
use axum::Router;
use std::sync::Arc;

type ApiRouter = Router<Arc<AppState>>;

pub(crate) fn mount_project_routes(router: ApiRouter) -> ApiRouter {
    let router = mount_project_base_routes(router);
    let router = mount_project_runtime_routes(router);
    let router = mount_project_lifecycle_routes(router);
    let router = mount_project_goal_task_routes(router);
    mount_project_planning_routes(router)
}

fn mount_project_base_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/projects",
            axum::routing::get(crate::project_list_routes::list_projects)
                .post(crate::project_create_routes::create_project),
        )
        .route(
            "/api/projects/launch",
            axum::routing::post(crate::project_launch_routes::launch_project),
        )
        .route(
            "/api/projects/environment",
            axum::routing::get(crate::project_environment_routes::projects_environment),
        )
        .route(
            "/api/projects/github/status",
            axum::routing::get(crate::project_github_routes::github_status),
        )
        .route(
            "/api/projects/github/token",
            axum::routing::put(crate::project_github_routes::configure_github_token)
                .delete(crate::project_github_routes::delete_github_token),
        )
        .route(
            "/api/projects/github/repos",
            axum::routing::get(crate::project_github_routes::github_repositories),
        )
}

fn mount_project_runtime_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/projects/{id}/runtime",
            axum::routing::get(crate::project_runtime_routes::project_runtime),
        )
        .route(
            "/api/projects/{id}/runtime/start",
            axum::routing::post(crate::project_runtime_start_routes::start_project_runtime),
        )
        .route(
            "/api/projects/{id}/runtime/pause",
            axum::routing::post(crate::project_runtime_pause_routes::pause_project_runtime),
        )
        .route(
            "/api/projects/{id}/runtime/resume",
            axum::routing::post(crate::project_runtime_resume_routes::resume_project_runtime),
        )
        .route(
            "/api/projects/{id}/runtime/answer",
            axum::routing::post(crate::project_answer_routes::answer_project_ask),
        )
        .route(
            "/api/projects/{id}/runtime/tool-request",
            axum::routing::post(crate::project_tool_request_routes::respond_project_tool_request),
        )
        .route(
            "/api/projects/{id}/runtime/takeover",
            axum::routing::post(crate::project_runtime_takeover_routes::takeover_project_runtime),
        )
}

fn mount_project_lifecycle_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/projects/{slug}",
            axum::routing::get(crate::project_detail_routes::get_project_by_slug)
                .patch(crate::project_update_routes::update_project)
                .delete(crate::project_delete_routes::delete_project),
        )
        .route(
            "/api/projects/{id}/archive",
            axum::routing::post(crate::project_archive_routes::archive_project),
        )
        .route(
            "/api/projects/{id}/resume",
            axum::routing::get(crate::project_resume_routes::resume_project),
        )
        .route(
            "/api/projects/{id}/lifecycle",
            axum::routing::patch(crate::project_lifecycle_routes::set_project_lifecycle_phase),
        )
}

fn mount_project_goal_task_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/projects/{id}/goals",
            axum::routing::get(crate::project_goal_routes::list_project_goals)
                .post(crate::project_goal_routes::create_project_goal),
        )
        .route(
            "/api/projects/{id}/goals/{goal_id}",
            axum::routing::delete(crate::project_goal_routes::delete_project_goal)
                .patch(crate::project_goal_routes::update_project_goal),
        )
        .route(
            "/api/projects/{id}/goals/{goal_id}/pause",
            axum::routing::post(crate::project_goal_routes::pause_project_goal),
        )
        .route(
            "/api/projects/{id}/goals/{goal_id}/resume",
            axum::routing::post(crate::project_goal_routes::resume_project_goal),
        )
        .route(
            "/api/projects/{id}/tasks",
            axum::routing::get(crate::project_task_routes::list_project_tasks)
                .post(crate::project_task_routes::create_project_task),
        )
        .route(
            "/api/project-tasks/{id}",
            axum::routing::patch(crate::project_task_routes::update_project_task)
                .delete(crate::project_task_routes::delete_project_task),
        )
}

fn mount_project_planning_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/projects/{id}/milestones",
            axum::routing::get(crate::project_milestone_routes::list_milestones)
                .post(crate::project_milestone_routes::create_milestone),
        )
        .route(
            "/api/projects/{id}/milestones/progress",
            axum::routing::get(crate::project_milestone_routes::get_milestone_progress),
        )
        .route(
            "/api/milestones/{id}/complete",
            axum::routing::post(crate::project_milestone_routes::complete_milestone),
        )
        .route(
            "/api/projects/{id}/checkpoints",
            axum::routing::get(crate::project_checkpoint_routes::list_checkpoints)
                .post(crate::project_checkpoint_routes::create_checkpoint),
        )
        .route(
            "/api/active-project/{agent_id}",
            axum::routing::get(crate::project_active_routes::get_active_project)
                .put(crate::project_active_routes::set_active_project)
                .delete(crate::project_active_routes::clear_active_project),
        )
}
