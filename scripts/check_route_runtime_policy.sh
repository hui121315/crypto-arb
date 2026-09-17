#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"
REGISTRY="$ROOT/crates/api/src/route_specs.rs"
FIXTURE="$ROOT/test/e2e/fixtures/route_runtime_policy.mjs"
SPEC="$ROOT/test/e2e/route_registry.spec.ts"
RUNTIME_SMOKE="$ROOT/scripts/verify_api_security_runtime_smoke.sh"

for file in "$INVENTORY" "$REGISTRY" "$FIXTURE" "$SPEC" "$RUNTIME_SMOKE"; do
  if [[ ! -s "$file" ]]; then
    printf 'route runtime policy gate failed: missing %s\n' "${file#$ROOT/}" >&2
    exit 1
  fi
done

node --input-type=module - "$INVENTORY" "$REGISTRY" "$FIXTURE" "$SPEC" "$RUNTIME_SMOKE" <<'NODE'
import fs from "node:fs";
import { pathToFileURL } from "node:url";

const [inventoryPath, registryPath, fixturePath, specPath, runtimeSmokePath] = process.argv.slice(2);
const fail = (message) => {
  console.error(`route runtime policy gate failed: ${message}`);
  process.exit(1);
};

const fields = [
  "class",
  "defaultExposure",
  "risk",
  "authPolicy",
  "auditPolicy",
  "featureFlag",
];
const inventoryLines = fs.readFileSync(inventoryPath, "utf8").trim().split("\n");
const header = inventoryLines.shift()?.split("\t");
if (!header || header.join("\t") !== "path\tmethods\trouter\tclass\tdefault_exposure\trisk\tauth_policy\taudit_policy\tfeature_flag\tshared_dto\tfrontend_client\towner\tnotes") {
  fail("route inventory header drifted");
}

const inventory = new Map();
const alwaysOnBearerInventory = [];
for (const line of inventoryLines) {
  const row = line.split("\t");
  if (row.length !== header.length) fail(`inventory row has ${row.length} columns: ${line}`);
  for (const method of row[1].split(",")) {
    const route = {
      class: row[3],
      defaultExposure: row[4],
      risk: row[5],
      authPolicy: row[6],
      auditPolicy: row[7],
      featureFlag: row[8],
      method,
      path: row[0],
    };
    inventory.set(`${method} ${row[0]}`, route);
    if (route.defaultExposure === "always" && route.authPolicy === "bearer") {
      alwaysOnBearerInventory.push(route);
    }
  }
}

const fixture = await import(pathToFileURL(fixturePath).href);
const policy = fixture.ROUTE_RUNTIME_POLICY;
const alwaysOnBearerPolicy = fixture.ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY;
const runtimeSmokeSource = fs.readFileSync(runtimeSmokePath, "utf8");
const expectedLabels = [
  "core_status",
  "disabled_spot_diagnostic",
  "disabled_strategy_diagnostic",
  "high_risk_action",
  "high_risk_secret",
  "high_risk_execution",
];
if (!Array.isArray(policy) || policy.length !== expectedLabels.length) {
  fail(`fixture must declare exactly ${expectedLabels.length} route policy records`);
}
if (!Array.isArray(alwaysOnBearerPolicy) || alwaysOnBearerPolicy.length === 0) {
  fail("fixture must declare the always-on bearer route matrix");
}
if (!fixture.LOCAL_FIXTURE_TOKEN?.includes("fixture") || !fixture.LOCAL_FIXTURE_SECRET?.includes("fixture")) {
  fail("fixture credentials must remain obvious local sentinels");
}

if (policy.map((row) => row.label).join(",") !== expectedLabels.join(",")) {
  fail(`fixture labels must be ${expectedLabels.join(",")}`);
}

for (const route of policy) {
  const actual = inventory.get(`${route.method} ${route.path}`);
  if (!actual) fail(`fixture route missing inventory row: ${route.method} ${route.path}`);
  for (const field of fields) {
    if (route[field] !== actual[field]) {
      fail(`${route.method} ${route.path} ${field} fixture=${route[field]} inventory=${actual[field]}`);
    }
  }
}

const inventoryMatrixKeys = alwaysOnBearerInventory.map((route) => `${route.method} ${route.path}`);
const fixtureMatrixKeys = alwaysOnBearerPolicy.map((route) => `${route.method} ${route.path}`);
if (fixtureMatrixKeys.length !== inventoryMatrixKeys.length) {
  fail(`always-on bearer fixture count=${fixtureMatrixKeys.length} inventory count=${inventoryMatrixKeys.length}`);
}
if (new Set(fixtureMatrixKeys).size !== fixtureMatrixKeys.length) {
  fail("always-on bearer fixture contains duplicate method/path entries");
}
if (fixtureMatrixKeys.join("\n") !== inventoryMatrixKeys.join("\n")) {
  fail("always-on bearer fixture no longer follows inventory method/path order");
}
for (const route of alwaysOnBearerPolicy) {
  const actual = inventory.get(`${route.method} ${route.path}`);
  if (!actual) fail(`always-on bearer fixture route missing inventory row: ${route.method} ${route.path}`);
  for (const field of fields) {
    if (route[field] !== actual[field]) {
      fail(`${route.method} ${route.path} ${field} fixture=${route[field]} inventory=${actual[field]}`);
    }
  }
}
for (const routeClass of ["main_p0", "diagnostic", "legacy", "readiness"]) {
  if (!alwaysOnBearerPolicy.some((route) => route.class === routeClass)) {
    fail(`always-on bearer fixture no longer covers ${routeClass}`);
  }
}

const [core, spotDiagnostic, strategyDiagnostic, highRisk, highRiskSecret, highRiskExecution] = policy;
if (core.authPolicy !== "bearer" || core.defaultExposure !== "always" || core.featureFlag !== "core") {
  fail("core metadata fixture no longer declares the authenticated always-on surface");
}
if (
  spotDiagnostic.class !== "diagnostic"
  || spotDiagnostic.defaultExposure !== "default_off"
  || spotDiagnostic.featureFlag !== "api_surface.spot_v1"
) {
  fail("spot diagnostic fixture no longer declares a disabled api_surface route");
}
if (
  strategyDiagnostic.class !== "diagnostic"
  || strategyDiagnostic.defaultExposure !== "default_off"
  || strategyDiagnostic.featureFlag !== "api_surface.strategy_v1"
) {
  fail("strategy diagnostic fixture no longer declares a disabled api_surface route");
}
if (highRisk.risk !== "high" || highRisk.authPolicy !== "bearer" || highRisk.auditPolicy !== "action_run") {
  fail("high-risk fixture no longer declares bearer action-run audit policy");
}
if (
  highRiskSecret.risk !== "high"
  || highRiskSecret.authPolicy !== "bearer"
  || highRiskSecret.auditPolicy !== "secret_mutation"
) {
  fail("secret fixture no longer declares bearer secret-mutation audit policy");
}
if (
  highRiskExecution.risk !== "high"
  || highRiskExecution.authPolicy !== "bearer"
  || highRiskExecution.auditPolicy !== "action_run"
) {
  fail("execution fixture no longer declares bearer action-run audit policy");
}

const registry = fs.readFileSync(registryPath, "utf8");
const registryConstructor = (route) => {
  const hasActionRun = ["action_run", "secret_mutation"].includes(route.auditPolicy);
  const constructor = hasActionRun
    ? `${route.method.toLowerCase()}_action_run`
    : route.method.toLowerCase();
  const fieldsPattern = hasActionRun
    ? [route.path, route.class, route.defaultExposure, route.risk, route.auditPolicy]
    : [route.path, route.class, route.defaultExposure, route.risk, route.authPolicy, route.auditPolicy];
  const pattern = fieldsPattern.map((field) => `\\s*"${field.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}"\\s*,?`).join("");
  return new RegExp(`RouteEndpointSpec::${constructor}\\(${pattern}`, "s");
};
for (const route of policy) {
  if (!registryConstructor(route).test(registry)) {
    fail(`RouteEndpointSpec runtime metadata missing ${route.method} ${route.path}`);
  }
}
if (!/Self::SpotV1\s*=>\s*"api_surface\.spot_v1"/.test(registry)) {
  fail("RouteGate::SpotV1 no longer maps to api_surface.spot_v1");
}

const fixtureSource = fs.readFileSync(fixturePath, "utf8");
const specSource = fs.readFileSync(specPath, "utf8");
if (/\btest\.(?:skip|fixme)\b|\btest\.describe\.skip\b/.test(specSource)) {
  fail("route registry Playwright spec must not skip its local contract");
}
for (const marker of [
  "startRouteRuntimeFixture",
  "page.evaluate",
  "ROUTE_DISABLED",
  "CREDENTIAL_PERMISSION_DENIED",
  "actionRunId",
  "idempotencyKey",
  "redactedFields",
  "LOCAL_FIXTURE_SECRET",
]) {
  if (!specSource.includes(marker)) fail(`route registry spec missing ${marker} assertion boundary`);
}
if (
  !fixtureSource.includes("redactedFields")
  || !fixtureSource.includes("ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY")
  || !fixtureSource.includes("API_ROUTE_INVENTORY.tsv")
  || fixtureSource.includes("process.env")
) {
  fail("route runtime fixture must record redaction and must not read environment credentials");
}

for (const marker of [
  "assert_default_off_surface_routes",
  "assert_always_on_bearer_auth_matrix",
  "assert_always_on_bearer_cors_matrix",
  "assert_always_on_bearer_read_matrix",
  "API_ROUTE_INVENTORY.tsv",
  "runtime-default-off-",
  "runtime-always-on-auth-",
  "APP_API_SURFACE__STRATEGY_V1",
]) {
  if (!runtimeSmokeSource.includes(marker)) {
    fail(`API binary route-surface smoke missing ${marker} boundary`);
  }
}

for (const marker of ["ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY", "routeRuntimeMatrix", "every always-on bearer route"]) {
  if (!`${fixtureSource}\n${specSource}`.includes(marker)) {
    fail(`route registry browser matrix missing ${marker} boundary`);
  }
}

console.log("OK route runtime policy gate (core metadata, disabled alias, full always-on bearer auth/CORS/read matrix, redaction, audit, binary default-off matrix)");
NODE
