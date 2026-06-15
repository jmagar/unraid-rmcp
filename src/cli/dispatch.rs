use anyhow::Result;

use unraid_mcp::app::UnraidService;

use super::commands::CliCommand;
use super::format::print_human;

pub async fn run(service: &UnraidService, cmd: CliCommand, json: bool) -> Result<()> {
    let (label, data) = match cmd {
        CliCommand::Array => ("array", service.array().await?),
        CliCommand::Disks => ("disks", service.disks().await?),
        CliCommand::Docker => ("docker", service.docker().await?),
        CliCommand::DockerLogs { ref id, tail } => {
            ("docker_logs", service.docker_logs(id, tail).await?)
        }
        CliCommand::Vms => ("vms", service.vms().await?),
        CliCommand::Server => ("server", service.server().await?),
        CliCommand::Info => ("info", service.info().await?),
        CliCommand::Shares => ("shares", service.shares().await?),
        CliCommand::Notifications => ("notifications", service.notifications().await?),
        CliCommand::LogFiles => ("log_files", service.log_files().await?),
        CliCommand::LogFile {
            ref path,
            lines,
            start_line,
        } => ("log_file", service.log_file(path, lines, start_line).await?),
        CliCommand::Services => ("services", service.services().await?),
        CliCommand::Network => ("network", service.network().await?),
        CliCommand::Ups => ("ups", service.ups().await?),
        CliCommand::UpsConfig => ("ups_config", service.ups_config().await?),
        CliCommand::Metrics => ("metrics", service.metrics().await?),
        CliCommand::Plugins => ("plugins", service.plugins().await?),
        CliCommand::ParityHistory => ("parity_history", service.parity_history().await?),
        CliCommand::Vars => ("vars", service.vars().await?),
        CliCommand::Registration => ("registration", service.registration().await?),
        CliCommand::Flash => ("flash", service.flash().await?),
        CliCommand::Rclone => ("rclone", service.rclone().await?),
        CliCommand::RemoteAccess => ("remote_access", service.remote_access().await?),
        CliCommand::Connect => ("connect", service.connect().await?),
        CliCommand::Online => ("online", service.online().await?),
        CliCommand::SystemTime => ("system_time", service.system_time().await?),
        CliCommand::InstalledUnraidPlugins => (
            "installed_unraid_plugins",
            service.installed_unraid_plugins().await?,
        ),
        CliCommand::IsSsoEnabled => ("is_sso_enabled", service.is_sso_enabled().await?),
        CliCommand::PublicOidcProviders => (
            "public_oidc_providers",
            service.public_oidc_providers().await?,
        ),
        CliCommand::OidcProviders => ("oidc_providers", service.oidc_providers().await?),
        CliCommand::OidcConfiguration => {
            ("oidc_configuration", service.oidc_configuration().await?)
        }
        CliCommand::ApiKeys => ("api_keys", service.api_keys().await?),
        CliCommand::ApiKeyPossibleRoles => (
            "api_key_possible_roles",
            service.api_key_possible_roles().await?,
        ),
        CliCommand::ApiKeyPossiblePermissions => (
            "api_key_possible_permissions",
            service.api_key_possible_permissions().await?,
        ),
        CliCommand::GetAvailableAuthActions => (
            "get_available_auth_actions",
            service.get_available_auth_actions().await?,
        ),
        CliCommand::GetApiKeyCreationFormSchema => (
            "get_api_key_creation_form_schema",
            service.get_api_key_creation_form_schema().await?,
        ),
        CliCommand::Config => ("config", service.config().await?),
        CliCommand::Settings => ("settings", service.settings().await?),
        CliCommand::Display => ("display", service.display().await?),
        CliCommand::Customization => ("customization", service.customization().await?),
        CliCommand::InternalBootContext => (
            "internal_boot_context",
            service.internal_boot_context().await?,
        ),
        CliCommand::Me => ("me", service.me().await?),
        CliCommand::Owner => ("owner", service.owner().await?),
        CliCommand::Servers => ("servers", service.servers().await?),
        CliCommand::IsFreshInstall => ("is_fresh_install", service.is_fresh_install().await?),
        CliCommand::PublicTheme => ("public_theme", service.public_theme().await?),
        CliCommand::NetworkInterfaces => {
            ("network_interfaces", service.network_interfaces().await?)
        }
        CliCommand::TimeZoneOptions => ("time_zone_options", service.time_zone_options().await?),
        CliCommand::AssignableDisks => ("assignable_disks", service.assignable_disks().await?),
        CliCommand::PluginInstallOperations => (
            "plugin_install_operations",
            service.plugin_install_operations().await?,
        ),
        CliCommand::Cloud => ("cloud", service.cloud().await?),
        CliCommand::ApiKey(id) => ("api_key", service.api_key(&id).await?),
        CliCommand::Disk(id) => ("disk", service.disk(&id).await?),
        CliCommand::OidcProvider(id) => ("oidc_provider", service.oidc_provider(&id).await?),
        CliCommand::UpsDeviceById(id) => ("ups_device_by_id", service.ups_device_by_id(&id).await?),
        CliCommand::PluginInstallOperation(id) => (
            "plugin_install_operation",
            service.plugin_install_operation(&id).await?,
        ),
        CliCommand::ValidateOidcSession(token) => (
            "validate_oidc_session",
            service.validate_oidc_session(&token).await?,
        ),
        CliCommand::GetPermissionsForRoles(roles) => (
            "get_permissions_for_roles",
            service.get_permissions_for_roles(&roles).await?,
        ),
        // Doctor and setup are intercepted in main.rs before reaching dispatch.
        CliCommand::Doctor | CliCommand::Setup(_) => {
            unreachable!("doctor/setup are handled before service construction")
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        print_human(label, &data);
    }
    Ok(())
}
