use super::*;

pub(super) fn api_status_slot_body(
    operation_health: Memo<Option<VenueOperationHealthSnapshot>>,
    operation_problem: Memo<Option<ApiProblem>>,
    environment: Memo<Option<ExecutionEnvironment>>,
) -> impl IntoView {
    let readiness = Memo::new(move |_| {
        category_readiness(
            RuntimeCategory::Api,
            operation_health.get().as_ref(),
            operation_problem.get().as_ref(),
            environment.get(),
        )
        .readiness
    });
    view! {
        <div
            data-testid="status-api-runtime"
            data-state=move || readiness.get().state()
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
                readiness.get().dot_class()
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
