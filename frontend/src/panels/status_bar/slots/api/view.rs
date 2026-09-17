use super::*;

pub(super) fn api_status_slot_body(
    operation_health: Memo<Option<VenueOperationHealthSnapshot>>,
    operation_problem: Memo<Option<ApiProblem>>,
    environment: Memo<Option<ExecutionEnvironment>>,
) -> impl IntoView {
    view! {
        <div
            data-testid="status-api-runtime"
            class=move || {
                let operation_health = operation_health.get();
                let operation_problem = operation_problem.get();
                api_slot_class_for_environment(
                    operation_health.as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                )
            }
            title=move || {
                let operation_health = operation_health.get();
                let operation_problem = operation_problem.get();
                api_title_for_environment(
                    operation_health.as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                )
            }
        >
            <span class=move || {
                let operation_health = operation_health.get();
                let operation_problem = operation_problem.get();
                dot_class(api_degraded_for_environment(
                    operation_health.as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                ))
            }></span>
            <span class="slot-label">{API_SLOT_LABEL}</span>
            <span class="num">{move || {
                let operation_health = operation_health.get();
                let operation_problem = operation_problem.get();
                api_label_with_problem_for_environment(
                    operation_health.as_ref(),
                    operation_problem.as_ref(),
                    environment.get(),
                )
            }}</span>
        </div>
    }
}
