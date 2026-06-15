//! Typed GraphQL operations (cynic) — **spike**.
//!
//! These mirror a couple of the hand-written queries in `graphql.rs`, but are
//! defined as Rust types checked against the vendored Unraid SDL at *compile
//! time* (see `build.rs`). The client runs them and serialises the typed result
//! straight back to `serde_json::Value`, so the rest of the stack (dispatch, CLI
//! formatters, MCP output, pagination) is unchanged — that's the migration
//! strategy under evaluation: typed at the wire, `Value` downstream.
//!
//! `#[cynic(...)]` drives the GraphQL mapping; `#[serde(...)]` drives the JSON we
//! emit downstream — both set to camelCase so the output matches the GraphQL
//! field names the formatters already expect.

/// The schema module the cynic derives resolve types against. Backed by the SDL
/// registered as `"unraid"` in `build.rs`.
#[cynic::schema("unraid")]
mod schema {}

// ── custom scalars (serialise transparently to their inner JSON type) ────────

// cynic's `Scalar` derive already provides the serde impls (it serialises
// transparently to the inner value — so `BigInt` stays a JSON string).
/// `BigInt` is delivered as a JSON string; keep it a string end to end.
#[derive(cynic::Scalar, Clone, Debug)]
pub struct BigInt(pub String);

#[derive(cynic::Scalar, Clone, Debug)]
pub struct PrefixedID(pub String);

#[derive(cynic::Scalar, Clone, Debug)]
pub struct DateTime(pub String);

/// `JSON` scalar — arbitrary JSON; wrap `serde_json::Value` so objects/arrays
/// round-trip (not just strings).
#[derive(cynic::Scalar, Clone, Debug)]
#[cynic(graphql_type = "JSON")]
pub struct Json(pub serde_json::Value);

// ── flash ────────────────────────────────────────────────────────────────────

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct FlashQuery {
    pub flash: Flash,
}

// `guid` is intentionally omitted — it is non-null in the SDL but null at runtime.
#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flash {
    pub id: PrefixedID,
    pub vendor: String,
    pub product: String,
}

// ── online / system_time / installed_unraid_plugins (first new-action batch) ──

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
pub struct OnlineQuery {
    pub online: bool,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct SystemTimeQuery {
    pub system_time: SystemTime,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct SystemTime {
    pub current_time: String,
    pub time_zone: String,
    pub use_ntp: bool,
    pub ntp_servers: Vec<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginsQuery {
    pub installed_unraid_plugins: Vec<String>,
}

// ── array (the stress test: nesting, lists, BigInt, 5 enums) ─────────────────
//
// Note: cynic structs map to a *selection*, not a type — so `parities`, `disks`,
// and `caches` (different field subsets of the same `ArrayDisk` type) each need
// their own struct. That selection-not-type rule is the main verbosity cost of
// the migration. All response parsing here is serde (`from_value`), so every
// type derives serde *and* the cynic derive that checks it against the SDL.

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct ArrayQuery {
    pub array: UnraidArray,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct UnraidArray {
    pub state: ArrayState,
    pub capacity: ArrayCapacity,
    pub parity_check_status: ParityCheck,
    pub parities: Vec<ArrayDiskParity>,
    pub disks: Vec<ArrayDiskData>,
    pub caches: Vec<ArrayDiskCache>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArrayCapacity {
    pub kilobytes: Capacity,
    pub disks: Capacity,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
pub struct Capacity {
    pub free: String,
    pub used: String,
    pub total: String,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParityCheck {
    pub status: ParityCheckStatus,
    pub running: Option<bool>,
    pub progress: Option<i32>,
    pub speed: Option<String>,
    pub errors: Option<i32>,
    pub correcting: Option<bool>,
    pub paused: Option<bool>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "ArrayDisk", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ArrayDiskParity {
    pub id: PrefixedID,
    pub name: Option<String>,
    pub device: Option<String>,
    pub size: Option<BigInt>,
    pub status: Option<ArrayDiskStatus>,
    pub temp: Option<i32>,
    pub num_errors: Option<BigInt>,
    #[cynic(rename = "type")]
    pub r#type: ArrayDiskType,
    pub is_spinning: Option<bool>,
    pub rotational: Option<bool>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "ArrayDisk", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ArrayDiskData {
    pub id: PrefixedID,
    pub name: Option<String>,
    pub device: Option<String>,
    pub size: Option<BigInt>,
    pub status: Option<ArrayDiskStatus>,
    pub temp: Option<i32>,
    pub num_errors: Option<BigInt>,
    pub num_reads: Option<BigInt>,
    pub num_writes: Option<BigInt>,
    pub fs_size: Option<BigInt>,
    pub fs_free: Option<BigInt>,
    pub fs_used: Option<BigInt>,
    #[cynic(rename = "type")]
    pub r#type: ArrayDiskType,
    pub color: Option<ArrayDiskFsColor>,
    pub is_spinning: Option<bool>,
    pub rotational: Option<bool>,
    pub fs_type: Option<String>,
    pub comment: Option<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "ArrayDisk", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ArrayDiskCache {
    pub id: PrefixedID,
    pub name: Option<String>,
    pub device: Option<String>,
    pub size: Option<BigInt>,
    pub status: Option<ArrayDiskStatus>,
    pub temp: Option<i32>,
    pub num_errors: Option<BigInt>,
    pub fs_size: Option<BigInt>,
    pub fs_free: Option<BigInt>,
    pub fs_used: Option<BigInt>,
    #[cynic(rename = "type")]
    pub r#type: ArrayDiskType,
    pub color: Option<ArrayDiskFsColor>,
    pub is_spinning: Option<bool>,
    pub rotational: Option<bool>,
    pub fs_type: Option<String>,
}

// ── oidc: public providers / providers / configuration / isSSOEnabled ─────────

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
pub struct IsSsoEnabledQuery {
    #[cynic(rename = "isSSOEnabled")]
    #[serde(rename = "isSSOEnabled")]
    pub is_sso_enabled: bool,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct PublicOidcProvidersQuery {
    pub public_oidc_providers: Vec<PublicOidcProvider>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct PublicOidcProvider {
    pub id: cynic::Id, // SDL: ID!
    pub name: String,
    pub button_text: Option<String>,
    pub button_icon: Option<String>,
    pub button_variant: Option<String>,
    pub button_style: Option<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct OidcProvidersQuery {
    pub oidc_providers: Vec<OidcProvider>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct OidcProvider {
    pub id: PrefixedID,
    pub name: String,
    pub client_id: String,
    pub issuer: Option<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub jwks_uri: Option<String>,
    pub scopes: Vec<String>,
    pub authorization_rules: Option<Vec<OidcAuthorizationRule>>,
    pub authorization_rule_mode: Option<AuthorizationRuleMode>,
    pub button_text: Option<String>,
    pub button_icon: Option<String>,
    pub button_variant: Option<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcAuthorizationRule {
    pub claim: String,
    pub operator: AuthorizationOperator,
    pub value: Vec<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct OidcConfigurationQuery {
    pub oidc_configuration: OidcConfiguration,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct OidcConfiguration {
    pub providers: Vec<OidcProvider>,
    pub default_allowed_origins: Option<Vec<String>>,
}

// ── auth: api keys / roles / permissions / auth actions / form schema ─────────

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ApiKeysQuery {
    pub api_keys: Vec<ApiKey>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ApiKey {
    pub id: PrefixedID,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub roles: Vec<Role>,
    pub created_at: String,
    pub permissions: Vec<Permission>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
pub struct Permission {
    pub resource: Resource,
    pub actions: Vec<AuthAction>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyPossibleRolesQuery {
    pub api_key_possible_roles: Vec<Role>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyPossiblePermissionsQuery {
    pub api_key_possible_permissions: Vec<Permission>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct AvailableAuthActionsQuery {
    pub get_available_auth_actions: Vec<AuthAction>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query", rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyCreationFormSchemaQuery {
    pub get_api_key_creation_form_schema: ApiKeyFormSettings,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyFormSettings {
    pub id: PrefixedID,
    pub data_schema: Json,
    pub ui_schema: Json,
    pub values: Json,
}

// `Resource` is hand-written (not via gql_enum!) because one SDL value,
// `CONNECT__REMOTE_ACCESS`, has a double underscore the macro can't emit.
#[derive(cynic::Enum, Clone, Copy, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum Resource {
    ActivationCode,
    ApiKey,
    Array,
    Cloud,
    Config,
    Connect,
    #[cynic(rename = "CONNECT__REMOTE_ACCESS")]
    ConnectRemoteAccess,
    Customizations,
    Dashboard,
    Disk,
    Display,
    Docker,
    Flash,
    Info,
    Logs,
    Me,
    Network,
    Notifications,
    Online,
    Os,
    Owner,
    Permission,
    Registration,
    Servers,
    Services,
    Share,
    Vars,
    Vms,
    Welcome,
}

// ── config / settings / display / customization / internalBootContext ─────────

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct ConfigQuery {
    pub config: Config,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub id: PrefixedID,
    pub valid: Option<bool>,
    pub error: Option<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct SettingsQuery {
    pub settings: Settings,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub id: PrefixedID,
    pub unified: UnifiedSettings,
    pub api: ApiConfig,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSettings {
    pub id: PrefixedID,
    pub data_schema: Json,
    pub ui_schema: Json,
    pub values: Json,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct ApiConfig {
    pub version: String,
    pub extra_origins: Vec<String>,
    pub sandbox: Option<bool>,
    pub sso_sub_ids: Vec<String>,
    pub plugins: Vec<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct DisplayQuery {
    pub display: InfoDisplay,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoDisplay {
    pub id: PrefixedID,
    pub case: InfoDisplayCase,
    pub theme: ThemeName,
    pub unit: Temperature,
    pub scale: bool,
    pub tabs: bool,
    pub resize: bool,
    pub wwn: bool,
    pub total: bool,
    pub usage: bool,
    pub text: bool,
    pub warning: i32,
    pub critical: i32,
    pub hot: i32,
    pub max: Option<i32>,
    pub locale: Option<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoDisplayCase {
    pub id: PrefixedID,
    pub url: String,
    pub icon: String,
    pub error: String,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct CustomizationQuery {
    pub customization: Option<Customization>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct Customization {
    pub activation_code: Option<ActivationCode>,
    pub onboarding: Onboarding,
    pub available_languages: Option<Vec<Language>>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationCode {
    pub code: Option<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct Onboarding {
    pub status: OnboardingStatus,
    pub is_partner_build: bool,
    pub completed: bool,
    pub completed_at_version: Option<String>,
    pub should_open: bool,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Language {
    pub code: String,
    pub name: String,
    pub url: Option<String>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(graphql_type = "Query")]
#[serde(rename_all = "camelCase")]
pub struct InternalBootContextQuery {
    pub internal_boot_context: OnboardingInternalBootContext,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct OnboardingInternalBootContext {
    pub array_stopped: bool,
    pub boot_eligible: Option<bool>,
    pub booted_from_flash_with_internal_boot_setup: bool,
    pub enable_boot_transfer: Option<String>,
    pub reserved_names: Vec<String>,
    pub share_names: Vec<String>,
    pub pool_names: Vec<String>,
    pub drive_warnings: Vec<OnboardingInternalBootDriveWarning>,
}

#[derive(cynic::QueryFragment, serde::Serialize)]
#[cynic(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct OnboardingInternalBootDriveWarning {
    pub disk_id: String,
    pub device: String,
    pub warnings: Vec<String>,
}

/// `ThemeName` SDL values are lowercase, so per-variant renames (the macro can't).
#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum ThemeName {
    #[cynic(rename = "azure")]
    Azure,
    #[cynic(rename = "black")]
    Black,
    #[cynic(rename = "gray")]
    Gray,
    #[cynic(rename = "white")]
    White,
}

// ── enums (cynic checks them vs the SDL; serde does the JSON round-trip) ──────

macro_rules! gql_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        // Variant names mirror the SDL's enum values (e.g. `DISK_*`), so the
        // shared-prefix lint is a false positive here.
        #[derive(cynic::Enum, Clone, Copy, Debug)]
        #[allow(clippy::enum_variant_names)]
        pub enum $name { $($variant),+ }
    };
}

gql_enum!(ArrayState {
    Started,
    Stopped,
    NewArray,
    ReconDisk,
    DisableDisk,
    SwapDsbl,
    InvalidExpansion,
    ParityNotBiggest,
    TooManyMissingDisks,
    NewDiskTooSmall,
    NoDataDisks,
});
gql_enum!(ParityCheckStatus {
    NeverRun,
    Running,
    Paused,
    Completed,
    Cancelled,
    Failed
});
gql_enum!(ArrayDiskStatus {
    DiskNp,
    DiskOk,
    DiskNpMissing,
    DiskInvalid,
    DiskWrong,
    DiskDsbl,
    DiskNpDsbl,
    DiskDsblNew,
    DiskNew,
});
gql_enum!(ArrayDiskType {
    Data,
    Parity,
    Boot,
    Flash,
    Cache
});
gql_enum!(ArrayDiskFsColor {
    GreenOn,
    GreenBlink,
    BlueOn,
    BlueBlink,
    YellowOn,
    YellowBlink,
    RedOn,
    RedOff,
    GreyOff,
});

// OIDC authorization-rule enums (oidc batch).
gql_enum!(AuthorizationOperator {
    Equals,
    Contains,
    EndsWith,
    StartsWith,
});
gql_enum!(AuthorizationRuleMode { Or, And });

// Auth enums (auth batch).
gql_enum!(Role {
    Admin,
    Connect,
    Guest,
    Viewer
});
gql_enum!(AuthAction {
    CreateAny,
    CreateOwn,
    ReadAny,
    ReadOwn,
    UpdateAny,
    UpdateOwn,
    DeleteAny,
    DeleteOwn,
});

// Config/onboarding enums (config batch).
gql_enum!(OnboardingStatus {
    Incomplete,
    Upgrade,
    Downgrade,
    Completed
});
gql_enum!(Temperature {
    Celsius,
    Fahrenheit
});
