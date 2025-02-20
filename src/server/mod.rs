pub mod context;
pub mod endpoints;

use std::{env, sync::Arc};

use anyhow::{anyhow, Result};
use dropshot::{ApiDescription, ConfigDropshot, HttpServerStarter};
use log::info;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};

use crate::server::context::Context;

/// Create an API description for the server.
pub fn create_api_description() -> Result<ApiDescription<Arc<Context>>> {
    fn register_endpoints(api: &mut ApiDescription<Arc<Context>>) -> Result<(), String> {
        api.register(crate::server::endpoints::ping).unwrap();
        api.register(crate::server::endpoints::api_get_schema).unwrap();

        /* Discord OAuth2 */
        api.register(crate::server::endpoints::listen_auth_discord_callback)
            .unwrap();
        api.register(crate::server::endpoints::listen_auth_discord_consent)
            .unwrap();

        Ok(())
    }

    // Describe the API.
    let tag_config = serde_json::from_str(include_str!("../../openapi/tag-config.json")).unwrap();
    let mut api = ApiDescription::new().tag_config(tag_config);

    if let Err(err) = register_endpoints(&mut api) {
        panic!("failed to register entrypoints: {}", err);
    }

    Ok(api)
}

pub async fn create_server(
    s: &crate::Server,
    opts: &crate::Opts,
) -> Result<(dropshot::HttpServer<Arc<Context>>, Arc<Context>)> {
    let mut api = create_api_description()?;
    let schema = get_openapi(&mut api)?;

    let config_dropshot = ConfigDropshot {
        bind_address: s.address.parse()?,
        default_request_body_max_bytes: 107374182400, // 100 Gigiabytes.
        default_handler_task_mode: dropshot::HandlerTaskMode::CancelOnDisconnect,
        log_headers: Default::default(),
    };

    let logger = opts.create_logger("server");
    let dropshot_logger = logger.new(slog::o!("component" => "dropshot"));

    let api_context = Arc::new(Context::new(schema, logger, s.clone()).await?);

    let server = HttpServerStarter::new(&config_dropshot, api, api_context.clone(), &dropshot_logger)
        .map_err(|error| anyhow!("failed to create server: {}", error))?
        .start();

    Ok((server, api_context))
}

/// Get the OpenAPI specification for the server.
pub fn get_openapi(api: &mut ApiDescription<Arc<Context>>) -> Result<serde_json::Value> {
    // Create the API schema.
    let mut definition = api.openapi(
        "KittyCAD Text to CAD Discord Bot",
        semver::Version::parse(clap::crate_version!()).expect("Crate version should be valid semver"),
    );
    definition
        .description("A discord bot to play with the KittyCAD Text to CAD API.")
        .contact_url("https://kittycad.io")
        .contact_email("discord@kittycad.io")
        .json()
        .map_err(|e| e.into())
}

pub async fn server(s: &crate::Server, opts: &crate::Opts) -> Result<()> {
    let (server, _api_context) = create_server(s, opts).await?;

    // For Cloud run & ctrl+c, shutdown gracefully.
    // "The main process inside the container will receive SIGTERM, and after a grace period,
    // SIGKILL."
    // Regsitering SIGKILL here will panic at runtime, so let's avoid that.
    let mut signals = Signals::new([SIGINT, SIGTERM])?;

    tokio::spawn(async move {
        if let Some(sig) = signals.forever().next() {
            info!("received signal: {:?}", sig);
            info!("triggering cleanup...");

            // Exit the process.
            info!("all clean, exiting!");
            std::process::exit(0);
        }
    });

    server.await.map_err(|error| anyhow!("server failed: {}", error))?;

    Ok(())
}
