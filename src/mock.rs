//! In-memory mock of the Unraid GraphQL upstream.
//!
//! The Rust side treats every GraphQL response as an opaque `serde_json::Value`,
//! so a faithful mock needs no GraphQL engine at all: it only has to recognise
//! *which* query is being asked and hand back a canned `data` payload. This
//! module provides that recognition ([`classify_query`]) plus the scenario
//! fixtures ([`Scenario`]) shared by both consumers:
//!
//!   * `examples/mock_unraid.rs` — a standalone HTTP server you point the real
//!     `runraid` binary / MCP transport / Claude skill at.
//!   * `tests/scenarios.rs` — mounts the same router on a wiremock server.
//!
//! Gated behind the `test-support` feature (and `cfg(test)`); it is never part
//! of a release build.

use std::collections::BTreeMap;

use graphql_parser::query::{Definition, OperationDefinition, Selection};
use serde_json::{Map, Value};

/// The realistic baseline. Every other scenario is a thin overlay on top of it.
const HEALTHY: &str = include_str!("../tests/fixtures/scenarios/healthy.json");
const DEGRADED: &str = include_str!("../tests/fixtures/scenarios/degraded.json");
const PARITY_RUNNING: &str = include_str!("../tests/fixtures/scenarios/parity-running.json");
const DISK_FAILING: &str = include_str!("../tests/fixtures/scenarios/disk-failing.json");

/// All scenario names known to the mock, in a stable order.
pub const SCENARIOS: &[&str] = &["healthy", "degraded", "parity-running", "disk-failing"];

/// Raw overlay JSON for a scenario name (the embedded fixture file), or `None`
/// if the name is unknown. `healthy` returns its own full fixture.
fn raw_overlay(name: &str) -> Option<&'static str> {
    match name {
        "healthy" => Some(HEALTHY),
        "degraded" => Some(DEGRADED),
        "parity-running" => Some(PARITY_RUNNING),
        "disk-failing" => Some(DISK_FAILING),
        _ => None,
    }
}

/// Map a raw GraphQL query string to the fixture key (the `action` name) that
/// serves it, by **parsing the query AST** and routing on the operation's root
/// field name (e.g. `upsDevices` → `ups`). This is robust to whitespace, field
/// reordering, and comments — unlike substring matching. The only root field
/// shared by two actions is `docker`, disambiguated by its sub-selection
/// (`logs` → `docker_logs`, otherwise `docker`).
///
/// Returns `None` if the query doesn't parse or its root field is unknown — the
/// server turns that into a GraphQL `errors` response, exactly as a real server
/// would for an unknown field.
pub fn classify_query(query: &str) -> Option<&'static str> {
    let (root, subfields) = root_field(query)?;
    Some(match root {
        "array" => "array",
        "disks" => "disks",
        "docker" if subfields.contains(&"logs") => "docker_logs",
        "docker" => "docker",
        "vms" => "vms",
        "server" => "server",
        "info" => "info",
        "shares" => "shares",
        "notifications" => "notifications",
        "logFiles" => "log_files",
        "logFile" => "log_file",
        "services" => "services",
        "network" => "network",
        "upsDevices" => "ups",
        "upsConfiguration" => "ups_config",
        "metrics" => "metrics",
        "plugins" => "plugins",
        "parityHistory" => "parity_history",
        "vars" => "vars",
        "registration" => "registration",
        "flash" => "flash",
        "rclone" => "rclone",
        "remoteAccess" => "remote_access",
        "connect" => "connect",
        _ => return None,
    })
}

/// Parse `query` and return its first operation's root field name together with
/// that field's immediate sub-field names. `None` if the query doesn't parse or
/// has no field selection at the root.
fn root_field(query: &str) -> Option<(&str, Vec<&str>)> {
    let doc = graphql_parser::parse_query::<&str>(query).ok()?;
    doc.definitions.iter().find_map(|def| {
        let selection_set = match def {
            Definition::Operation(OperationDefinition::Query(q)) => &q.selection_set,
            Definition::Operation(OperationDefinition::SelectionSet(s)) => s,
            Definition::Operation(OperationDefinition::Mutation(m)) => &m.selection_set,
            Definition::Operation(OperationDefinition::Subscription(s)) => &s.selection_set,
            Definition::Fragment(_) => return None,
        };
        let field = selection_set.items.iter().find_map(|s| match s {
            Selection::Field(f) => Some(f),
            _ => None,
        })?;
        let subfields = field
            .selection_set
            .items
            .iter()
            .filter_map(|s| match s {
                Selection::Field(f) => Some(f.name),
                _ => None,
            })
            .collect();
        Some((field.name, subfields))
    })
}

/// A fully-resolved scenario: fixture key → the `data` payload returned for that
/// query (already the object that belongs under GraphQL's top-level `data`).
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    payloads: BTreeMap<String, Value>,
}

impl Scenario {
    /// Resolve a scenario by name: parse the `healthy` baseline, then replace
    /// any fixture keys the named overlay provides. Keys beginning with `_`
    /// (e.g. `_note`) are treated as documentation and skipped.
    ///
    /// Returns `None` for an unknown scenario name.
    pub fn load(name: &str) -> Option<Self> {
        let overlay_src = raw_overlay(name)?;

        let mut payloads = parse_fixture(HEALTHY);
        if name != "healthy" {
            for (key, value) in parse_fixture(overlay_src) {
                payloads.insert(key, value);
            }
        }

        Some(Self {
            name: name.to_string(),
            payloads,
        })
    }

    /// The `data` payload for a fixture key, if present.
    pub fn payload(&self, key: &str) -> Option<&Value> {
        self.payloads.get(key)
    }

    /// Resolve a raw GraphQL query directly to a full response body
    /// (`{"data": …}`), or a GraphQL `errors` body when the query is not
    /// recognised. This is the whole server behaviour in one call.
    pub fn respond(&self, query: &str) -> Value {
        match classify_query(query).and_then(|key| self.payload(key)) {
            Some(data) => serde_json::json!({ "data": data }),
            None => serde_json::json!({
                "errors": [{
                    "message": "mock-unraid: no fixture matches this query",
                    "extensions": { "code": "MOCK_UNKNOWN_QUERY" }
                }]
            }),
        }
    }
}

/// Parse a fixture file into fixture-key → payload, dropping `_`-prefixed keys.
fn parse_fixture(src: &str) -> BTreeMap<String, Value> {
    let map: Map<String, Value> =
        serde_json::from_str(src).expect("embedded scenario fixture must be valid JSON");
    map.into_iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_loads_and_inherits_baseline() {
        for name in SCENARIOS {
            let s = Scenario::load(name).unwrap_or_else(|| panic!("scenario {name} should load"));
            // Inherited-from-baseline keys are always present, even in overlays
            // that don't mention them.
            assert!(s.payload("info").is_some(), "{name} missing inherited info");
            assert!(s.payload("array").is_some(), "{name} missing array");
        }
    }

    #[test]
    fn unknown_scenario_is_none() {
        assert!(Scenario::load("does-not-exist").is_none());
    }

    #[test]
    fn baseline_has_every_known_action() {
        // Each classifiable action must resolve to a payload in the baseline,
        // otherwise the standalone server would 'errors' on a real query.
        let healthy = Scenario::load("healthy").unwrap();
        let expected = [
            "array",
            "disks",
            "docker",
            "docker_logs",
            "vms",
            "server",
            "info",
            "shares",
            "notifications",
            "log_files",
            "log_file",
            "services",
            "network",
            "ups",
            "ups_config",
            "metrics",
            "plugins",
            "parity_history",
            "vars",
            "registration",
            "flash",
            "rclone",
            "remote_access",
            "connect",
        ];
        for key in expected {
            assert!(
                healthy.payload(key).is_some(),
                "baseline fixture is missing the `{key}` payload"
            );
        }
    }

    #[test]
    fn classify_routes_overlapping_queries() {
        // docker_logs vs docker
        assert_eq!(
            classify_query("query($id: PrefixedID!, $tail: Int) { docker { logs(id: $id, tail: $tail) { lines } } }"),
            Some("docker_logs")
        );
        assert_eq!(
            classify_query("query { docker { containers { id names } } }"),
            Some("docker")
        );
        // log_file vs log_files
        assert_eq!(
            classify_query("query($path: String!) { logFile(path: $path) { content } }"),
            Some("log_file")
        );
        assert_eq!(
            classify_query("query { logFiles { name path } }"),
            Some("log_files")
        );
        // connect vs remote_access (substring overlap)
        assert_eq!(
            classify_query("query { connect { id dynamicRemoteAccess { error } } }"),
            Some("connect")
        );
        assert_eq!(
            classify_query("query { remoteAccess { accessType forwardType port } }"),
            Some("remote_access")
        );
        // array vs disks (both mention nested `disks {`)
        assert_eq!(
            classify_query("query { array { state parityCheckStatus { status } disks { id } } }"),
            Some("array")
        );
        assert_eq!(
            classify_query("query { disks { id smartStatus temperature } }"),
            Some("disks")
        );
    }

    #[test]
    fn unknown_query_yields_errors_body() {
        let healthy = Scenario::load("healthy").unwrap();
        let body = healthy.respond("query { somethingNobodyAskedFor }");
        assert!(body.get("errors").is_some());
        assert!(body.get("data").is_none());
    }

    #[test]
    fn degraded_overrides_array_but_keeps_info() {
        let healthy = Scenario::load("healthy").unwrap();
        let degraded = Scenario::load("degraded").unwrap();
        // info is inherited verbatim
        assert_eq!(degraded.payload("info"), healthy.payload("info"));
        // array differs (a disk is disabled)
        assert_ne!(degraded.payload("array"), healthy.payload("array"));
    }
}
