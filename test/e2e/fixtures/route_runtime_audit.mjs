export function createRouteRuntimeAuditRecorder(auditEvents, requestId) {
  function recordAuditPair({
    action,
    actionRunId,
    idempotencyKey,
    orderIds = [],
    outcome,
    redactedFields,
    request,
    runIds = [],
  }) {
    const common = {
      action,
      actionRunId,
      actor: "api-token:local-route-runtime",
      idempotencyKey,
      ...(orderIds.length > 0 ? { orderIds } : {}),
      redactedFields,
      requestId: requestId(request),
      ...(runIds.length > 0 ? { runIds } : {}),
    };
    auditEvents.push(
      { ...common, outcome: "accepted" },
      { ...common, outcome },
    );
  }

  function recordDeniedAudit({ request, route }) {
    auditEvents.push({
      action: route.action,
      actionKind: route.actionKind,
      actor: "unknown",
      actorKind: "unknown",
      method: request.method,
      outcome: "denied",
      path: route.path,
      problemCode: "UNAUTHORIZED",
      requestId: requestId(request),
      resource: route.path,
      resourceKind: route.resourceKind,
      status: 401,
    });
  }

  return Object.freeze({ recordAuditPair, recordDeniedAudit });
}
