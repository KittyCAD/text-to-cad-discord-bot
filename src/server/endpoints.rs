use std::sync::Arc;

use anyhow::Result;
use dropshot::{endpoint, HttpError, HttpResponseAccepted, HttpResponseOk, Query, RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::server::context::Context;

/**
 * Return the OpenAPI schema in JSON format.
 */
#[endpoint {
    method = GET,
    path = "/",
}]
pub async fn api_get_schema(
    rqctx: RequestContext<Arc<Context>>,
) -> Result<HttpResponseOk<serde_json::Value>, HttpError> {
    Ok(HttpResponseOk(rqctx.context().schema.clone()))
}

/// The response from the `/ping` endpoint.
#[derive(Deserialize, Debug, JsonSchema, Serialize)]
pub struct Pong {
    /// The pong response.
    pub message: String,
}

/** Return pong. */
#[endpoint {
    method = GET,
    path = "/ping",
}]
pub async fn ping(_rqctx: RequestContext<Arc<Context>>) -> Result<HttpResponseOk<Pong>, HttpError> {
    Ok(HttpResponseOk(Pong {
        message: "pong".to_string(),
    }))
}

#[derive(Debug, Clone, Default, JsonSchema, Deserialize, Serialize)]
pub struct UserConsentURL {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Clone, Default, JsonSchema, Deserialize, Serialize)]
pub struct DiscordAuthCallback {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub guild_id: String,
}

/** Get the consent URL for Discord auth. */
#[endpoint {
    method = GET,
    path = "/auth/discord/consent",
}]
pub async fn listen_auth_discord_consent(
    rqctx: RequestContext<Arc<Context>>,
) -> Result<HttpResponseOk<UserConsentURL>, HttpError> {
    let ctx = rqctx.context();
    Ok(HttpResponseOk(UserConsentURL {
        url: format!("https://discord.com/api/oauth2/authorize?client_id={}&permissions=8&redirect_uri={}&response_type=code&scope={}&permissions=70368744177655",
                 ctx.settings.discord_client_id,
                 ctx.settings.discord_redirect_uri,
                 [
                    "identify",
                    "email",
                    "bot",
                    "guilds",
                    "guilds.members.read",
                    "webhook.incoming",
                 ].join("%20"),
        ),
    }))
}

/** Listen for callbacks to Discord auth. */
#[endpoint {
    method = GET,
    path = "/auth/discord/callback",
}]
pub async fn listen_auth_discord_callback(
    rqctx: RequestContext<Arc<Context>>,
    query_args: Query<DiscordAuthCallback>,
) -> Result<HttpResponseAccepted<String>, HttpError> {
    let ctx = rqctx.context();
    if let Err(e) = handle_auth_discord_callback(ctx, query_args).await {
        return Err(HttpError::for_internal_error(e.to_string()));
    }

    Ok(HttpResponseAccepted("ok".to_string()))
}

pub async fn handle_auth_discord_callback(ctx: &Arc<Context>, query_args: Query<DiscordAuthCallback>) -> Result<()> {
    let event = query_args.into_inner();

    let mut headers = reqwest::header::HeaderMap::new();
    headers.append(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    let params = [
        ("grant_type", "authorization_code"),
        ("code", &event.code),
        ("client_id", &ctx.settings.discord_client_id),
        ("client_secret", &ctx.settings.discord_client_secret),
        ("redirect_uri", &ctx.settings.discord_redirect_uri),
    ];
    let client = reqwest::Client::new();
    let resp = client
        .post("https://discord.com/api/oauth2/token")
        .headers(headers)
        .form(&params)
        .basic_auth(
            &ctx.settings.discord_client_id,
            Some(&ctx.settings.discord_client_secret),
        )
        .send()
        .await?;

    slog::info!(ctx.logger, "discord auth callback response: {:?}", resp.text().await?);

    Ok(())
}
