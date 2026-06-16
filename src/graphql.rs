use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use thiserror::Error;

use crate::config::UnraidConfig;

/// Typed classification of a failure talking to the upstream Unraid GraphQL API.
///
/// This is the *routing* contract: the dispatch layer downcasts an [`anyhow::Error`]
/// to this enum to decide how to surface the failure to the MCP caller, instead of
/// matching on message prose. Each variant carries only a category-level message —
/// raw upstream bodies / GraphQL `errors` are logged server-side, never embedded
/// here (see `query_with_vars`), so they cannot leak to the caller or aid schema
/// probing.
#[derive(Debug, Error)]
pub enum UpstreamError {
    /// Could not reach the upstream (connection refused, DNS, TLS handshake, timeout).
    #[error("{0}")]
    Unreachable(String),
    /// Upstream rejected our credentials (HTTP 401/403).
    #[error("{0}")]
    Auth(String),
    /// Any other upstream-side failure (5xx, GraphQL errors, malformed response).
    #[error("{0}")]
    Other(String),
}

#[derive(Clone)]
pub struct UnraidClient {
    client: Client,
    url: String,
    api_key: String,
}

impl UnraidClient {
    pub fn new(cfg: &UnraidConfig) -> Result<Self> {
        if cfg.api_url.is_empty() {
            anyhow::bail!("UNRAID_API_URL is not set");
        }
        if cfg.api_key.is_empty() {
            anyhow::bail!("UNRAID_API_KEY is not set");
        }
        if cfg.skip_tls_verify {
            tracing::warn!(
                "UNRAID_API_SKIP_TLS_VERIFY is enabled: TLS certificate verification is DISABLED. \
                 The API key is sent to an unverified endpoint and is exposed to on-path (MITM) \
                 attackers. Only use this for self-signed certificates on a trusted network."
            );
        }
        let client = reqwest::ClientBuilder::new()
            .danger_accept_invalid_certs(cfg.skip_tls_verify)
            .connect_timeout(Duration::from_secs(5))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            client,
            url: cfg.api_url.clone(),
            api_key: cfg.api_key.clone(),
        })
    }

    async fn query(&self, gql: &str) -> Result<Value> {
        self.query_with_vars(gql, Value::Null).await
    }

    /// Same as [`query`], but sends GraphQL `variables` alongside the query so
    /// caller-controlled values never have to be interpolated into the query text.
    async fn query_with_vars(&self, gql: &str, variables: Value) -> Result<Value> {
        self.send_graphql(json!({ "query": gql, "variables": variables }))
            .await
    }

    /// POST an already-assembled GraphQL HTTP body (`{query, variables, …}`),
    /// returning the `data` object. Shared by the string-query path and the typed
    /// (cynic) path so both get identical auth headers, timeout, and the
    /// category-level error mapping that never leaks raw upstream bodies.
    async fn send_graphql(&self, body: Value) -> Result<Value> {
        let span = tracing::info_span!("graphql.query", url = %self.url);
        let _guard = span.enter();

        let resp = match self
            .client
            .post(&self.url)
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(Duration::from_secs(30))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                // Transport-level failure: connection refused, DNS, TLS, timeout.
                // The reqwest error text can carry OS-locale-specific wording, so it
                // is logged server-side; the caller gets a stable category message.
                tracing::warn!(error = %e, "GraphQL request failed at the transport layer");
                let category = if e.is_connect() || e.is_timeout() {
                    "upstream unreachable — is UNRAID_API_URL reachable?"
                } else {
                    "upstream request failed before a response was received"
                };
                return Err(UpstreamError::Unreachable(category.to_string()).into());
            }
        };

        let status = resp.status();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // Read the body only to log it; never return it to the caller.
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(status = %status, body = %body, "upstream rejected the request (auth)");
            return Err(UpstreamError::Auth("upstream rejected the request — check that UNRAID_API_KEY is correct and has not expired".to_string()).into());
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(status = %status, body = %body, "upstream returned a non-success HTTP status");
            return Err(UpstreamError::Other(format!("upstream returned HTTP {status}")).into());
        }

        let body: Value = match resp.json().await {
            Ok(body) => body,
            Err(e) => {
                tracing::error!(error = %e, "failed to parse GraphQL response as JSON");
                return Err(UpstreamError::Other(
                    "upstream returned an unexpected non-JSON response".to_string(),
                )
                .into());
            }
        };

        if let Some(errors) = body.get("errors") {
            // Log the full GraphQL `errors` array server-side; the caller only learns
            // that a GraphQL-level error occurred (no field names / messages leaked).
            tracing::error!(errors = %errors, "upstream returned GraphQL errors");
            return Err(UpstreamError::Other(
                "upstream GraphQL error — see server logs for details".to_string(),
            )
            .into());
        }

        let data = body.get("data").cloned().ok_or_else(|| {
            tracing::error!(body = %body, "GraphQL response missing 'data' field");
            UpstreamError::Other("upstream GraphQL response missing 'data' field".to_string())
        })?;
        tracing::debug!(status = %status, "GraphQL query ok");
        Ok(data)
    }

    /// Expose the HTTP client and URL for the health probe.
    pub fn raw_client(&self) -> (&Client, &str, &str) {
        (&self.client, &self.url, &self.api_key)
    }

    /// Run a typed cynic operation over the existing transport: serialise the
    /// operation to the GraphQL HTTP body, send it, then deserialise the `data`
    /// back through the cynic type and re-emit it as `Value` (so dispatch /
    /// formatters / MCP are unchanged). cynic checks the query against the SDL at
    /// compile time; this validates the *response* round-trips through that type.
    async fn run_typed<T, V>(&self, op: cynic::Operation<T, V>) -> Result<Value>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + 'static,
        V: serde::Serialize,
    {
        let data = self.send_graphql(serde_json::to_value(&op)?).await?;
        let typed: T = serde_json::from_value(data).map_err(|e| {
            tracing::error!(error = %e, "upstream response did not match the typed schema");
            UpstreamError::Other("upstream response shape did not match the schema".to_string())
        })?;
        Ok(serde_json::to_value(typed)?)
    }

    /// Typed-client spike: `flash` via cynic instead of a hand-written string.
    pub async fn flash_typed(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::FlashQuery::build(()))
            .await
    }

    /// Typed-client spike: `array` via cynic (nesting / lists / BigInt / enums).
    pub async fn array_typed(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::ArrayQuery::build(()))
            .await
    }

    pub async fn online(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::OnlineQuery::build(()))
            .await
    }

    pub async fn system_time(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::SystemTimeQuery::build(()))
            .await
    }

    pub async fn installed_unraid_plugins(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::InstalledPluginsQuery::build(()))
            .await
    }

    pub async fn is_sso_enabled(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::IsSsoEnabledQuery::build(()))
            .await
    }

    pub async fn public_oidc_providers(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::PublicOidcProvidersQuery::build(()))
            .await
    }

    pub async fn oidc_providers(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::OidcProvidersQuery::build(()))
            .await
    }

    pub async fn oidc_configuration(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::OidcConfigurationQuery::build(()))
            .await
    }

    pub async fn api_keys(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::ApiKeysQuery::build(()))
            .await
    }

    pub async fn api_key_possible_roles(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::ApiKeyPossibleRolesQuery::build(()))
            .await
    }

    pub async fn api_key_possible_permissions(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::ApiKeyPossiblePermissionsQuery::build(()))
            .await
    }

    pub async fn get_available_auth_actions(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::AvailableAuthActionsQuery::build(()))
            .await
    }

    pub async fn get_api_key_creation_form_schema(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::ApiKeyCreationFormSchemaQuery::build(()))
            .await
    }

    pub async fn config(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::ConfigQuery::build(()))
            .await
    }

    pub async fn settings(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::SettingsQuery::build(()))
            .await
    }

    pub async fn display(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::DisplayQuery::build(()))
            .await
    }

    pub async fn customization(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::CustomizationQuery::build(()))
            .await
    }

    pub async fn internal_boot_context(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::InternalBootContextQuery::build(()))
            .await
    }

    pub async fn me(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::MeQuery::build(())).await
    }

    pub async fn owner(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::OwnerQuery::build(()))
            .await
    }

    pub async fn servers(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::ServersQuery::build(()))
            .await
    }

    pub async fn is_fresh_install(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::IsFreshInstallQuery::build(()))
            .await
    }

    pub async fn public_theme(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::PublicThemeQuery::build(()))
            .await
    }

    pub async fn network_interfaces(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::NetworkInterfacesQuery::build(()))
            .await
    }

    pub async fn time_zone_options(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::TimeZoneOptionsQuery::build(()))
            .await
    }

    pub async fn assignable_disks(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::AssignableDisksQuery::build(()))
            .await
    }

    pub async fn plugin_install_operations(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::PluginInstallOperationsQuery::build(()))
            .await
    }

    pub async fn cloud(&self) -> Result<Value> {
        use cynic::QueryBuilder;
        self.run_typed(crate::gql_typed::CloudQuery::build(()))
            .await
    }

    pub async fn api_key(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{ApiKeyByIdQuery, PrefixedID, PrefixedIdVars};
        use cynic::QueryBuilder;
        self.run_typed(ApiKeyByIdQuery::build(PrefixedIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn disk(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{DiskByIdQuery, PrefixedID, PrefixedIdVars};
        use cynic::QueryBuilder;
        self.run_typed(DiskByIdQuery::build(PrefixedIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn oidc_provider(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{OidcProviderByIdQuery, PrefixedID, PrefixedIdVars};
        use cynic::QueryBuilder;
        self.run_typed(OidcProviderByIdQuery::build(PrefixedIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn ups_device_by_id(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{StringIdVars, UpsDeviceByIdQuery};
        use cynic::QueryBuilder;
        self.run_typed(UpsDeviceByIdQuery::build(StringIdVars {
            id: id.to_string(),
        }))
        .await
    }

    pub async fn plugin_install_operation(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{OperationIdVars, PluginInstallOperationByIdQuery};
        use cynic::QueryBuilder;
        self.run_typed(PluginInstallOperationByIdQuery::build(OperationIdVars {
            operation_id: cynic::Id::new(id),
        }))
        .await
    }

    pub async fn validate_oidc_session(&self, token: &str) -> Result<Value> {
        use crate::gql_typed::{TokenVars, ValidateOidcSessionQuery};
        use cynic::QueryBuilder;
        self.run_typed(ValidateOidcSessionQuery::build(TokenVars {
            token: token.to_string(),
        }))
        .await
    }

    pub async fn get_permissions_for_roles(&self, roles: &[String]) -> Result<Value> {
        use crate::gql_typed::{PermissionsForRolesQuery, Role, RolesVars};
        use cynic::QueryBuilder;
        let roles: Vec<Role> = serde_json::from_value(serde_json::to_value(roles)?)
            .map_err(|e| UpstreamError::Other(format!("invalid role value: {e}")))?;
        self.run_typed(PermissionsForRolesQuery::build(RolesVars { roles }))
            .await
    }

    // ── mutations (write) ──
    pub async fn recalculate_overview(&self) -> Result<Value> {
        use cynic::MutationBuilder;
        self.run_typed(crate::gql_typed::RecalculateOverviewMutation::build(()))
            .await
    }

    pub async fn delete_archived_notifications(&self) -> Result<Value> {
        use cynic::MutationBuilder;
        self.run_typed(crate::gql_typed::DeleteArchivedNotificationsMutation::build(()))
            .await
    }

    pub async fn archive_notification(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{ArchiveNotificationMutation, PrefixedID, PrefixedIdVars};
        use cynic::MutationBuilder;
        self.run_typed(ArchiveNotificationMutation::build(PrefixedIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn create_notification(
        &self,
        title: &str,
        subject: &str,
        description: &str,
        importance: &str,
        link: Option<&str>,
    ) -> Result<Value> {
        use crate::gql_typed::{
            CreateNotificationMutation, CreateNotificationVars, NotificationData,
            NotificationImportance,
        };
        use cynic::MutationBuilder;
        let importance: NotificationImportance = serde_json::from_value(json!(importance))
            .map_err(|e| {
                UpstreamError::Other(format!(
                    "invalid importance (expected ALERT/INFO/WARNING): {e}"
                ))
            })?;
        let input = NotificationData {
            title: title.to_string(),
            subject: subject.to_string(),
            description: description.to_string(),
            importance,
            link: link.map(|s| s.to_string()),
        };
        self.run_typed(CreateNotificationMutation::build(CreateNotificationVars {
            input,
        }))
        .await
    }

    pub async fn vm_start(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{PrefixedID, VmIdVars, VmStartMutation};
        use cynic::MutationBuilder;
        self.run_typed(VmStartMutation::build(VmIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn vm_stop(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{PrefixedID, VmIdVars, VmStopMutation};
        use cynic::MutationBuilder;
        self.run_typed(VmStopMutation::build(VmIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn vm_pause(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{PrefixedID, VmIdVars, VmPauseMutation};
        use cynic::MutationBuilder;
        self.run_typed(VmPauseMutation::build(VmIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn vm_resume(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{PrefixedID, VmIdVars, VmResumeMutation};
        use cynic::MutationBuilder;
        self.run_typed(VmResumeMutation::build(VmIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn vm_force_stop(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{PrefixedID, VmForceStopMutation, VmIdVars};
        use cynic::MutationBuilder;
        self.run_typed(VmForceStopMutation::build(VmIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn vm_reboot(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{PrefixedID, VmIdVars, VmRebootMutation};
        use cynic::MutationBuilder;
        self.run_typed(VmRebootMutation::build(VmIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn vm_reset(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{PrefixedID, VmIdVars, VmResetMutation};
        use cynic::MutationBuilder;
        self.run_typed(VmResetMutation::build(VmIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn docker_start(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{DockerIdVars, DockerStartMutation, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(DockerStartMutation::build(DockerIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn docker_stop(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{DockerIdVars, DockerStopMutation, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(DockerStopMutation::build(DockerIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn docker_pause(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{DockerIdVars, DockerPauseMutation, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(DockerPauseMutation::build(DockerIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn docker_unpause(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{DockerIdVars, DockerUnpauseMutation, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(DockerUnpauseMutation::build(DockerIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn docker_update_container(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{DockerIdVars, DockerUpdateContainerMutation, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(DockerUpdateContainerMutation::build(DockerIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn docker_remove_container(
        &self,
        id: &str,
        with_image: Option<bool>,
    ) -> Result<Value> {
        use crate::gql_typed::{DockerRemoveContainerMutation, DockerRemoveVars, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(DockerRemoveContainerMutation::build(DockerRemoveVars {
            id: PrefixedID(id.to_string()),
            with_image,
        }))
        .await
    }

    pub async fn docker_update_containers(&self, ids: &[String]) -> Result<Value> {
        use crate::gql_typed::{DockerIdsVars, DockerUpdateContainersMutation, PrefixedID};
        use cynic::MutationBuilder;
        let ids = ids.iter().map(|i| PrefixedID(i.clone())).collect();
        self.run_typed(DockerUpdateContainersMutation::build(DockerIdsVars { ids }))
            .await
    }

    pub async fn docker_update_all_containers(&self) -> Result<Value> {
        use cynic::MutationBuilder;
        self.run_typed(crate::gql_typed::DockerUpdateAllContainersMutation::build(
            (),
        ))
        .await
    }

    pub async fn array_set_state(&self, desired_state: &str) -> Result<Value> {
        use crate::gql_typed::{
            ArraySetStateMutation, ArrayStateInput, ArrayStateInputState, ArrayStateInputVars,
        };
        use cynic::MutationBuilder;
        let desired_state: ArrayStateInputState = serde_json::from_value(json!(desired_state))
            .map_err(|e| {
                UpstreamError::Other(format!("invalid desired_state (START/STOP): {e}"))
            })?;
        let input = ArrayStateInput { desired_state };
        self.run_typed(ArraySetStateMutation::build(ArrayStateInputVars { input }))
            .await
    }

    pub async fn array_add_disk_to_array(&self, id: &str, slot: Option<i32>) -> Result<Value> {
        use crate::gql_typed::{
            ArrayAddDiskToArrayMutation, ArrayDiskInput, ArrayDiskInputVars, PrefixedID,
        };
        use cynic::MutationBuilder;
        let input = ArrayDiskInput {
            id: PrefixedID(id.to_string()),
            slot,
        };
        self.run_typed(ArrayAddDiskToArrayMutation::build(ArrayDiskInputVars {
            input,
        }))
        .await
    }

    pub async fn array_remove_disk_from_array(&self, id: &str, slot: Option<i32>) -> Result<Value> {
        use crate::gql_typed::{
            ArrayDiskInput, ArrayDiskInputVars, ArrayRemoveDiskFromArrayMutation, PrefixedID,
        };
        use cynic::MutationBuilder;
        let input = ArrayDiskInput {
            id: PrefixedID(id.to_string()),
            slot,
        };
        self.run_typed(ArrayRemoveDiskFromArrayMutation::build(
            ArrayDiskInputVars { input },
        ))
        .await
    }

    pub async fn array_mount_array_disk(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{ArrayDiskIdVars, ArrayMountArrayDiskMutation, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(ArrayMountArrayDiskMutation::build(ArrayDiskIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn array_unmount_array_disk(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{ArrayDiskIdVars, ArrayUnmountArrayDiskMutation, PrefixedID};
        use cynic::MutationBuilder;
        self.run_typed(ArrayUnmountArrayDiskMutation::build(ArrayDiskIdVars {
            id: PrefixedID(id.to_string()),
        }))
        .await
    }

    pub async fn array_clear_array_disk_statistics(&self, id: &str) -> Result<Value> {
        use crate::gql_typed::{
            ArrayClearArrayDiskStatisticsMutation, ArrayDiskIdVars, PrefixedID,
        };
        use cynic::MutationBuilder;
        self.run_typed(ArrayClearArrayDiskStatisticsMutation::build(
            ArrayDiskIdVars {
                id: PrefixedID(id.to_string()),
            },
        ))
        .await
    }

    pub async fn parity_check_start(&self, correct: bool) -> Result<Value> {
        use crate::gql_typed::{ParityCheckStartMutation, ParityCheckStartVars};
        use cynic::MutationBuilder;
        self.run_typed(ParityCheckStartMutation::build(ParityCheckStartVars {
            correct,
        }))
        .await
    }

    pub async fn parity_check_pause(&self) -> Result<Value> {
        use cynic::MutationBuilder;
        self.run_typed(crate::gql_typed::ParityCheckPauseMutation::build(()))
            .await
    }

    pub async fn parity_check_resume(&self) -> Result<Value> {
        use cynic::MutationBuilder;
        self.run_typed(crate::gql_typed::ParityCheckResumeMutation::build(()))
            .await
    }

    pub async fn parity_check_cancel(&self) -> Result<Value> {
        use cynic::MutationBuilder;
        self.run_typed(crate::gql_typed::ParityCheckCancelMutation::build(()))
            .await
    }

    // ── queries ───────────────────────────────────────────────────────────────

    pub async fn array(&self) -> Result<Value> {
        self.query(
            r#"query {
  array {
    state
    capacity {
      kilobytes { free used total }
      disks { free used total }
    }
    parityCheckStatus { status running progress speed errors correcting paused }
    parities { id name device size status temp numErrors type isSpinning rotational }
    disks {
      id name device size status temp numErrors numReads numWrites
      fsSize fsFree fsUsed type color isSpinning rotational fsType comment
    }
    caches {
      id name device size status temp numErrors
      fsSize fsFree fsUsed type color isSpinning rotational fsType
    }
  }
}"#,
        )
        .await
    }

    pub async fn disks(&self) -> Result<Value> {
        self.query(
            r#"query {
  disks {
    id device type name vendor size serialNum
    interfaceType smartStatus temperature isSpinning
    partitions { name fsType size }
  }
}"#,
        )
        .await
    }

    pub async fn docker(&self) -> Result<Value> {
        self.query(
            r#"query {
  docker {
    containers {
      id names image state status autoStart autoStartOrder
      ports { privatePort publicPort type ip }
      webUiUrl iconUrl isOrphaned isUpdateAvailable
    }
  }
}"#,
        )
        .await
    }

    pub async fn docker_logs(&self, container_id: &str, tail: Option<i64>) -> Result<Value> {
        let gql = r#"query($id: PrefixedID!, $tail: Int) {
  docker {
    logs(id: $id, tail: $tail) {
      containerId
      lines { timestamp message }
      cursor
    }
  }
}"#;
        self.query_with_vars(
            gql,
            json!({ "id": container_id, "tail": tail.unwrap_or(100) }),
        )
        .await
    }

    pub async fn vms(&self) -> Result<Value> {
        self.query(
            r#"query {
  vms {
    domains { id name state }
  }
}"#,
        )
        .await
    }

    pub async fn server(&self) -> Result<Value> {
        self.query(
            r#"query {
  server { id name comment status wanip lanip localurl remoteurl guid }
}"#,
        )
        .await
    }

    pub async fn info(&self) -> Result<Value> {
        self.query(
            r#"query {
  info {
    time
    os { platform distro release kernel arch hostname fqdn uptime }
    cpu { brand manufacturer cores threads speed speedmax socket }
    memory { layout { size type clockSpeed } }
    versions { core { unraid kernel } }
  }
}"#,
        )
        .await
    }

    pub async fn shares(&self) -> Result<Value> {
        self.query(
            r#"query {
  shares {
    id name free used size cache comment allocator luksStatus
  }
}"#,
        )
        .await
    }

    pub async fn notifications(&self) -> Result<Value> {
        self.query(
            r#"query {
  notifications {
    overview { unread { warning alert info total } archive { warning alert info total } }
    warningsAndAlerts { id title subject description importance type timestamp }
  }
}"#,
        )
        .await
    }

    pub async fn log_files(&self) -> Result<Value> {
        self.query(r#"query { logFiles { name path size modifiedAt } }"#)
            .await
    }

    pub async fn log_file(
        &self,
        path: &str,
        lines: Option<i64>,
        start_line: Option<i64>,
    ) -> Result<Value> {
        let gql = r#"query($path: String!, $lines: Int, $startLine: Int) {
  logFile(path: $path, lines: $lines, startLine: $startLine) {
    path content totalLines startLine
  }
}"#;
        self.query_with_vars(
            gql,
            json!({ "path": path, "lines": lines, "startLine": start_line }),
        )
        .await
    }

    pub async fn services(&self) -> Result<Value> {
        self.query(r#"query { services { id name online version uptime { timestamp } } }"#)
            .await
    }

    pub async fn network(&self) -> Result<Value> {
        self.query(r#"query { network { id accessUrls { type name ipv4 ipv6 } } }"#)
            .await
    }

    pub async fn ups(&self) -> Result<Value> {
        self.query(
            r#"query {
  upsDevices {
    id name model status
    battery { chargeLevel estimatedRuntime health }
    power { inputVoltage outputVoltage loadPercentage }
  }
}"#,
        )
        .await
    }

    pub async fn ups_config(&self) -> Result<Value> {
        self.query(
            r#"query {
  upsConfiguration {
    service upsCable upsType device batteryLevel minutes timeout
    killUps nisIp netServer upsName modelName
  }
}"#,
        )
        .await
    }

    pub async fn metrics(&self) -> Result<Value> {
        self.query(
            r#"query {
  metrics {
    cpu { percentTotal cpus { percentTotal percentUser percentSystem percentIdle } }
    memory {
      total used free available percentTotal
      swapTotal swapUsed swapFree percentSwapTotal
    }
    temperature {
      sensors { id name type location current { value unit } warning critical }
      summary { average warningCount criticalCount }
    }
  }
}"#,
        )
        .await
    }

    pub async fn plugins(&self) -> Result<Value> {
        self.query(r#"query { plugins { name version hasApiModule hasCliModule } }"#)
            .await
    }

    pub async fn parity_history(&self) -> Result<Value> {
        self.query(
            r#"query {
  parityHistory { date duration speed status errors progress correcting paused running }
}"#,
        )
        .await
    }

    pub async fn vars(&self) -> Result<Value> {
        self.query(
            r#"query {
  vars {
    version name timeZone comment sysModel
    useSsl port portssl useSsh portssh useTelnet porttelnet
    startArray spindownDelay shareSmbEnabled shareNfsEnabled shareAfpEnabled
    configValid configError regState regTo
    deviceCount flashGuid flashProduct flashVendor
    sbName sbVersion sbUpdated sbState
  }
}"#,
        )
        .await
    }

    pub async fn registration(&self) -> Result<Value> {
        self.query(r#"query { registration { id type state expiration updateExpiration } }"#)
            .await
    }

    pub async fn flash(&self) -> Result<Value> {
        // guid is non-nullable in schema but can be null at runtime — omit it
        self.query(r#"query { flash { id vendor product } }"#).await
    }

    pub async fn rclone(&self) -> Result<Value> {
        self.query(r#"query { rclone { remotes { name type } drives { name } } }"#)
            .await
    }

    pub async fn remote_access(&self) -> Result<Value> {
        self.query(r#"query { remoteAccess { accessType forwardType port } }"#)
            .await
    }

    pub async fn connect(&self) -> Result<Value> {
        self.query(
            r#"query {
  connect {
    id
    dynamicRemoteAccess { enabledType runningType error }
    settings { values { accessType forwardType port } }
  }
}"#,
        )
        .await
    }
}
