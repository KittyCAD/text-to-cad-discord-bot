//! A discord bot for interacting with the KittyCAD text-to-cad API.

#![deny(missing_docs)]

mod engine;
mod image;
#[macro_use]
mod enclose;
mod server;
#[cfg(test)]
mod tests;

use std::str::FromStr;

use anyhow::{bail, Result};
use clap::Parser;
use parse_display::{Display, FromStr};
use poise::serenity_prelude::{self as serenity, prelude::*, GatewayIntents};
use serde::{Deserialize, Serialize};
use slog::Drain;
use tracing_subscriber::{prelude::*, Layer};

struct Data {
    logger: slog::Logger,
    client: kittycad::Client,
}
type Context<'a> = poise::Context<'a, Data, anyhow::Error>;

lazy_static::lazy_static! {
/// Initialize the logger.
    // We need a slog::Logger for steno and when we export out the logs from re-exec-ed processes.
    pub static ref LOGGER: slog::Logger = {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_async::Async::new(drain).build().fuse();
        slog::Logger::root(drain, slog::slog_o!("app" => "discord"))
    };
}

/// This doc string acts as a help message when the user runs '--help'
/// as do all doc strings on fields.
#[derive(Parser, Debug, Clone)]
#[clap(version = clap::crate_version!(), author = clap::crate_authors!("\n"))]
pub struct Opts {
    /// Print debug info
    #[clap(short, long)]
    pub debug: bool,

    /// Print logs as json
    #[clap(short, long)]
    pub json: bool,

    /// The subcommand to run.
    #[clap(subcommand)]
    pub subcmd: SubCommand,
}

impl Opts {
    /// Setup our logger.
    pub fn create_logger(&self, app: &str) -> slog::Logger {
        if self.json {
            let drain = slog_json::Json::default(std::io::stderr()).fuse();
            self.async_root_logger(drain, app)
        } else {
            let decorator = slog_term::TermDecorator::new().build();
            let drain = slog_term::FullFormat::new(decorator).build().fuse();
            self.async_root_logger(drain, app)
        }
    }

    fn async_root_logger<T>(&self, drain: T, app: &str) -> slog::Logger
    where
        T: slog::Drain + Send + 'static,
        <T as slog::Drain>::Err: std::fmt::Debug,
    {
        let level = if self.debug {
            slog::Level::Debug
        } else {
            slog::Level::Info
        };

        let level_drain = slog::LevelFilter(drain, level).fuse();
        let async_drain = slog_async::Async::new(level_drain).build().fuse();
        slog::Logger::root(async_drain, slog::slog_o!("app" => app.to_owned()))
    }
}

/// A subcommand for our cli.
#[derive(Parser, Debug, Clone)]
pub enum SubCommand {
    /// Run the server.
    Server(Server),
    /// Convert a gltf file to an image.
    ConvertImage(ConvertImage),
    /// A subcommand for just getting an image for a prompt.
    TextToCad(TextToCad),
}

/// A subcommand for running the server.
#[derive(Parser, Clone, Debug)]
pub struct Server {
    /// IP address and port that the server should listen
    #[clap(short, long, default_value = "0.0.0.0:8080")]
    pub address: String,

    /// The discord bot token to use.
    #[clap(long, env = "DISCORD_TOKEN")]
    pub discord_token: String,

    /// The discord client ID to use.
    #[clap(long, env = "DISCORD_CLIENT_ID")]
    pub discord_client_id: String,

    /// The discord client secret to use.
    #[clap(long, env = "DISCORD_CLIENT_SECRET")]
    pub discord_client_secret: String,

    /// The discord redirect URI to use.
    #[clap(long, env = "DISCORD_REDIRECT_URI")]
    pub discord_redirect_uri: String,

    /// The KittyCAD API token to use.
    #[clap(long, env = "KITTYCAD_API_TOKEN")]
    pub kittycad_api_token: String,
}

/// A subcommand for converting a gltf file to an image.
#[derive(Parser, Clone, Debug)]
pub struct ConvertImage {
    /// Path to the gltf file.
    #[clap(short, long = "gltf-path")]
    pub gltf_path: std::path::PathBuf,

    /// Path to the output image file.
    #[clap(short, long = "image-path")]
    pub image_path: std::path::PathBuf,

    /// The KittyCAD API token to use.
    #[clap(long, env = "KITTYCAD_API_TOKEN")]
    pub kittycad_api_token: String,
}

/// A subcommand for just getting an image for a prompt.
#[derive(Parser, Clone, Debug)]
pub struct TextToCad {
    /// The prompt to use.
    #[clap(short, long)]
    pub prompt: String,

    /// The KittyCAD API token to use.
    #[clap(long, env = "KITTYCAD_API_TOKEN")]
    pub kittycad_api_token: String,
}

async fn on_error(error: poise::FrameworkError<'_, Data, anyhow::Error>) {
    // This is our custom error handler
    // They are many errors that can occur, so we only handle the ones we want to customize
    // and forward the rest to the default handler
    match error {
        poise::FrameworkError::Setup { error, .. } => panic!("Failed to start bot: {:?}", error),
        poise::FrameworkError::Command { error, ctx, .. } => {
            println!("Error in command `{}`: {:?}", ctx.command().name, error,);
        }
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                println!("Error while handling error: {}", e)
            }
        }
    }
}

/// Show help message.
#[poise::command(prefix_command, track_edits, slash_command, category = "Utility")]
async fn help(
    ctx: Context<'_>,
    #[description = "Command to get help for"]
    #[rest]
    mut command: Option<String>,
) -> Result<()> {
    // This makes it possible to just make `help` a subcommand of any command
    // `/fruit help` turns into `/help fruit`
    // `/fruit help apple` turns into `/help fruit apple`
    if ctx.invoked_command_name() != "help" {
        command = match command {
            Some(c) => Some(format!("{} {}", ctx.invoked_command_name(), c)),
            None => Some(ctx.invoked_command_name().to_string()),
        };
    }
    let extra_text_at_bottom = "\
Type `?help command` for more info on a command.
You can edit your `?help` message to the bot and the bot will edit its response.";

    let config = poise::samples::HelpConfiguration {
        show_subcommands: true,
        show_context_menu_commands: true,
        ephemeral: true,
        extra_text_at_bottom,

        ..Default::default()
    };
    poise::builtins::help(ctx, command.as_deref(), config).await?;
    Ok(())
}

/// The environment the server is running in.
#[derive(Display, FromStr, Copy, Eq, PartialEq, Debug, Deserialize, Serialize, Clone, Ord, PartialOrd)]
#[serde(rename_all = "UPPERCASE")]
#[display(style = "UPPERCASE")]
pub enum Environment {
    /// The development environment. This is for running locally.
    Development,
    /// The preview environment. This is when PRs are created and a service is deployed for testing.
    Preview,
    /// The production environment.
    Production,
}

impl Environment {
    /// Returns the current environment as parsed from the `SENTRY_ENV` environment variable.
    #[tracing::instrument]
    pub fn get() -> Self {
        match std::env::var("SENTRY_ENV") {
            Ok(en) => Self::from_str(&en.to_uppercase()).unwrap_or(Environment::Development),
            Err(_) => Environment::Development,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    let level_filter = if opts.debug {
        tracing_subscriber::filter::LevelFilter::DEBUG
    } else {
        tracing_subscriber::filter::LevelFilter::INFO
    };

    // Format fields using the provided closure.
    // We want to make this very consise otherwise the logs are not able to be read by humans.
    let format = tracing_subscriber::fmt::format::debug_fn(|writer, field, value| {
        if format!("{}", field) == "message" {
            write!(writer, "{}: {:?}", field, value)
        } else {
            write!(writer, "{}", field)
        }
    })
    // Separate each field with a comma.
    // This method is provided by an extension trait in the
    // `tracing-subscriber` prelude.
    .delimited(", ");

    let (json, plain) = if opts.json {
        // Cloud run likes json formatted logs if possible.
        // See: https://cloud.google.com/run/docs/logging
        // We could probably format these specifically for cloud run if we wanted,
        // will save that as a TODO: https://cloud.google.com/run/docs/logging#special-fields
        (
            Some(tracing_subscriber::fmt::layer().json().with_filter(level_filter)),
            None,
        )
    } else {
        (
            None,
            Some(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .fmt_fields(format)
                    .with_filter(level_filter),
            ),
        )
    };

    // Initialize the Sentry tracing.
    tracing_subscriber::registry().with(json).with(plain).init();

    if let Err(err) = run_cmd(&opts).await {
        bail!("running cmd `{:?}` failed: {:?}", &opts.subcmd, err);
    }

    Ok(())
}

async fn run_cmd(opts: &Opts) -> Result<()> {
    match &opts.subcmd {
        SubCommand::Server(s) => {
            // Run the dropshot server in the background.
            let handle = tokio::spawn(enclose! { (s, opts) async move {
                crate::server::server(&s, &opts).await?;
                Ok::<(), anyhow::Error>(())
            }});

            // Set up the framework.
            let framework = poise::Framework::builder()
                .options(poise::FrameworkOptions {
                    commands: vec![ping(), latency(), design(), help()],
                    prefix_options: poise::PrefixFrameworkOptions {
                        prefix: Some("?".into()),
                        ..Default::default()
                    },
                    // The global error handler for all error cases that may occur
                    on_error: |error| Box::pin(on_error(error)),
                    // This code is run before every command
                    pre_command: |ctx| {
                        Box::pin(async move {
                            slog::info!(
                                &ctx.data().logger,
                                "Got command '{}' by user '{}'",
                                ctx.command().qualified_name,
                                ctx.author().name,
                            );
                        })
                    },
                    // This code is run after a command if it was successful (returned Ok)
                    post_command: |ctx| {
                        Box::pin(async move {
                            slog::info!(
                                &ctx.data().logger,
                                "Processed command '{}'",
                                ctx.command().qualified_name
                            );
                        })
                    },
                    ..Default::default()
                })
                .setup(move |ctx, ready, framework| {
                    Box::pin(async move {
                        let logger = crate::LOGGER.clone();
                        slog::info!(&logger, "{} is connected!", ready.user.name);
                        poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                        Ok(Data {
                            logger,
                            client: kittycad::Client::new_from_env(),
                        })
                    })
                })
                .build();

            // For this example to run properly, the "Presence Intent" and "Server Members Intent"
            // options need to be enabled.
            // These are needed so the `required_permissions` macro works on the commands that need to
            // use it.
            // You will need to enable these 2 options on the bot application, and possibly wait up to 5
            // minutes.
            let intents = GatewayIntents::all();
            let mut client = Client::builder(&s.discord_token, intents).framework(framework).await?;

            // start listening for events by starting a single shard
            client.start_autosharded().await?;

            // If we get here, the client has disconnected cleanly.
            // So we should abort our handle for our server.
            handle.abort();
        }

        SubCommand::ConvertImage(c) => {
            let logger = opts.create_logger("convert-image");

            let users_client = kittycad::Client::new(&c.kittycad_api_token);

            // Read the contents of the file.
            let contents = tokio::fs::read(&c.gltf_path).await?;

            let image_bytes = crate::image::model_to_image(&logger, &users_client, &contents).await?;

            tokio::fs::write(&c.image_path, image_bytes).await?;
        }
        SubCommand::TextToCad(t) => {
            let logger = opts.create_logger("text-to-cad");

            let users_client = kittycad::Client::new(&t.kittycad_api_token);

            let mut model = get_model_for_prompt(&logger, &users_client, &t.prompt).await?;
            let image_file = get_image_bytes_for_model(&logger, &users_client, &model).await?;

            // Clear the outputs so we don't print them.
            model.outputs = Default::default();
            slog::info!(logger, "Model: {:?}", model);
            slog::info!(logger, "Image file: {:?}", image_file);
        }
    }

    Ok(())
}

/// Get information about this bot.
#[poise::command(slash_command, prefix_command)]
async fn about(ctx: Context<'_>) -> Result<()> {
    ctx.reply("A discord bot to play with the KittyCAD Text to CAD API. 🤪")
        .await?;

    Ok(())
}

/// Get the latency of the bot.
#[poise::command(slash_command, prefix_command)]
async fn latency(ctx: Context<'_>) -> Result<()> {
    ctx.reply(&format!("The shard latency is {:?}", ctx.ping().await))
        .await?;

    Ok(())
}

/// Ping the bot.
#[poise::command(slash_command, prefix_command)]
async fn ping(ctx: Context<'_>) -> Result<()> {
    ctx.reply("🏓").await?;

    Ok(())
}

/// Generate a CAD model from a text prompt.
#[poise::command(slash_command, prefix_command)]
async fn design(
    ctx: Context<'_>,
    #[rest]
    #[description = "Your design prompt"]
    prompt: String,
) -> Result<()> {
    let logger = &ctx.data().logger;

    // React to the message that we are working on it.
    // We want to use unicode eyes.
    ctx.reply('👀').await?;

    if prompt.trim().is_empty() {
        ctx.say("🫠 An argument is required to run this command.").await?;
        return Ok(());
    }

    if let Err(err) = run_text_to_cad_prompt(&ctx, prompt.trim()).await {
        // If the error was from the API, let's handle it better for each type of error.
        let e = match err.downcast_ref::<kittycad::types::error::Error>() {
            Some(kerr) => {
                if let kittycad::types::error::Error::Server { body, status } = kerr {
                    // Get the body of the response.
                    let error: kittycad::types::Error = serde_json::from_str(body).unwrap_or(kittycad::types::Error {
                        error_code: Some(status.to_string()),
                        message: Default::default(),
                        request_id: Default::default(),
                    });
                    let mut err_str = String::new();
                    if let Some(code) = &error.error_code {
                        err_str.push_str(&format!("{}: ", code));
                    }
                    err_str.push_str(&error.message);

                    err_str.trim().trim_end_matches(':').to_string()
                } else {
                    kerr.to_string()
                }
            }

            None => err.to_string(),
        };

        slog::warn!(logger, "Error running text to cad prompt: {}", e);
        let message = format!("🤮 {}", e);
        // TRuncate the message to the first 2000 characters.
        let message = &message[..std::cmp::min(message.len(), 2000)];

        if e.contains("User has not authenticated") {
            // Give a special emoji for this error.
            ctx.reply("🔒 We sent you a DM with instructions to login.").await?;
            // Send the message as a DM so we don't spam the channel.
            ctx.author()
                .direct_message(ctx.http(), serenity::builder::CreateMessage::new().content(message))
                .await?;

            // Return early so we don't send the message to the channel.
            return Ok(());
        }

        ctx.reply(format!("🤮 {}", message)).await?;
    }

    Ok(())
}

async fn run_text_to_cad_prompt(ctx: &Context<'_>, prompt: &str) -> Result<()> {
    let logger = &ctx.data().logger;
    let kittycad_client = &ctx.data().client;

    // Get a token for the user based on their discord id.
    let kittycad_token = kittycad_client
        .meta()
        .internal_get_api_token_for_discord_user(&ctx.author().id.get().to_string())
        .await?;
    // Now create a new kittycad client with the user's token.
    let users_client = kittycad::Client::new(kittycad_token.token);

    let model = get_model_for_prompt(logger, &users_client, prompt).await?;
    let image_bytes = match get_image_bytes_for_model(logger, &users_client, &model).await {
        Ok(bytes) => bytes,
        Err(err) => {
            slog::warn!(logger, "Error getting image bytes: {}", err);
            ctx.channel_id()
                .send_message(
                    &ctx.http(),
                    serenity::builder::CreateMessage::new().content(&format!(
                        "{}, you can login to view your model or give feedback at:
https://text-to-cad.zoo.dev/view/{}\n\nUnfortunately, we were unable to generate an image for your model. But you can still login to view it.\n\n```\n{}```\n",
                        ctx.author().mention(),
                        model.id,
                        err
                    )),
                )
                .await?;
            return Ok(());
        }
    };
    let image_name = format!("{}.png", model.id);

    let feedback_time_seconds = 120;
    let our_msg = ctx
        .channel_id()
        .send_message(
            &ctx.http(),
            serenity::builder::CreateMessage::new()
                .content(&format!(
                    "{}, you can login to view your model or give feedback at:
https://text-to-cad.zoo.dev/view/{}",
                    ctx.author().mention(),
                    model.id
                ))
                .embed(
                    serenity::builder::CreateEmbed::new()
                        .title(prompt)
                        .image(&format!("attachment://{}", image_name))
                        // Thumbs up or down emoji.
                        .footer(serenity::builder::CreateEmbedFooter::new(&format!(
                            r#"React with a 👍 or 👎 to this message to give feedback.
Feedback must be left within the next {} seconds."#,
                            feedback_time_seconds
                        )))
                        // Add a timestamp for the current time
                        // This also accepts a rfc3339 Timestamp
                        .timestamp(serenity::model::Timestamp::now()),
                )
                .add_file(serenity::builder::CreateAttachment::bytes(image_bytes, image_name)),
        )
        .await?;

    // Wait for feedback to the model based on emoji reaction.
    let collector = our_msg
        .await_reaction(ctx)
        .author_id(ctx.author().id)
        .timeout(std::time::Duration::from_secs(feedback_time_seconds))
        .filter(|reaction| {
            reaction.emoji == serenity::model::channel::ReactionType::Unicode("👍".to_string())
                || reaction.emoji == serenity::model::channel::ReactionType::Unicode("👎".to_string())
        })
        .await;

    if let Some(reaction) = collector {
        let emoji = reaction.emoji.as_data();
        let reaction = if emoji == "👍" {
            Some(kittycad::types::AiFeedback::ThumbsUp)
        } else if emoji == "👎" {
            Some(kittycad::types::AiFeedback::ThumbsDown)
        } else {
            None
        };

        if let Some(ai_reaction) = reaction {
            // Send our feedback on the model.
            users_client
                .ai()
                .create_text_to_cad_model_feedback(ai_reaction, model.id)
                .await?;
            // Let the user know we got their feedback.
            our_msg.react(ctx, '🧠').await?;
        }
    }
    // TODO: add export button.

    Ok(())
}

async fn get_model_for_prompt(
    logger: &slog::Logger,
    users_client: &kittycad::Client,
    prompt: &str,
) -> Result<kittycad::types::TextToCad> {
    slog::debug!(logger, "Got design request: {}", prompt);

    // Create the text-to-cad request.
    let mut model: kittycad::types::TextToCad = users_client
        .ai()
        .create_text_to_cad(
            kittycad::types::FileExportFormat::Gltf,
            &kittycad::types::TextToCadCreateBody {
                prompt: prompt.to_string(),
            },
        )
        .await?;

    slog::debug!(logger, "Got design response: {}", model);

    // Poll until the model is ready.
    let mut status = model.status.clone();
    // Get the current time.
    let start = std::time::Instant::now();
    // Give it 5 minutes to complete. That should be way
    // more than enough!
    while status != kittycad::types::ApiCallStatus::Completed
        && status != kittycad::types::ApiCallStatus::Failed
        && start.elapsed().as_secs() < 60 * 5
    {
        slog::debug!(logger, "Polling for design status: {}", status);

        // Poll for the status.
        let result = users_client
            .api_calls()
            .get_async_operation(&model.id.to_string())
            .await?;

        if let kittycad::types::AsyncApiCallOutput::TextToCad {
            completed_at,
            created_at,
            error,
            feedback,
            id,
            model_version,
            output_format,
            outputs,
            prompt,
            started_at,
            status,
            updated_at,
            user_id,
        } = result
        {
            model = kittycad::types::TextToCad {
                completed_at,
                created_at,
                error,
                feedback,
                id,
                model_version,
                output_format,
                outputs,
                prompt,
                started_at,
                status,
                updated_at,
                user_id,
            };
        } else {
            anyhow::bail!("Unexpected response type: {:?}", result);
        }

        status = model.status.clone();

        // Wait for a bit before polling again.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    // If the model failed we will want to tell the user.
    if model.status == kittycad::types::ApiCallStatus::Failed {
        if let Some(error) = model.error {
            slog::warn!(logger, "Design failed: {}", error);
            anyhow::bail!("Your prompt returned an error: ```\n{}\n```", error);
        } else {
            slog::warn!(logger, "Design failed: {:?}", model);
            anyhow::bail!("Your prompt returned an error, but no error message. :(");
        }
    }

    if model.status != kittycad::types::ApiCallStatus::Completed {
        slog::warn!(logger, "Design timed out: {:?}", model);

        anyhow::bail!("Your prompt timed out");
    }

    // Okay, we successfully got a model!
    slog::debug!(logger, "Design completed: {:?}", model.prompt);

    Ok(model)
}

async fn get_image_bytes_for_model(
    logger: &slog::Logger,
    users_client: &kittycad::Client,
    model: &kittycad::types::TextToCad,
) -> Result<Vec<u8>> {
    anyhow::bail!("Could not generate image");
    // Get the gltf bytes.
    let mut gltf_bytes = vec![];
    if let Some(outputs) = &model.outputs {
        for (key, value) in outputs {
            if key.ends_with(".gltf") {
                gltf_bytes = value.0.clone();
                break;
            }
        }
    } else {
        slog::warn!(logger, "Design completed, but no gltf outputs: {:?}", model);
        anyhow::bail!("Your design completed, but no gltf outputs were found");
    }

    let image_contents = crate::image::model_to_image(logger, users_client, &gltf_bytes).await?;

    Ok(image_contents)
}

#[cfg(test)]
mod test {
    use slog::Drain;

    use crate::{get_image_bytes_for_model, get_model_for_prompt};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_image_from_prompt() {
        let logger = {
            let decorator = slog_term::PlainDecorator::new(slog_term::TestStdoutWriter);
            let drain = std::sync::Mutex::new(slog_term::FullFormat::new(decorator).build()).fuse();

            slog::Logger::root(drain, slog::o!())
        };
        let mut kittycad_client = kittycad::Client::new_from_env();
        kittycad_client.set_base_url("https://api.dev.zoo.dev");

        let model = get_model_for_prompt(&logger, &kittycad_client, "a 2x4 lego")
            .await
            .unwrap();

        let image_bytes = get_image_bytes_for_model(&logger, &kittycad_client, &model)
            .await
            .unwrap();

        assert!(!image_bytes.is_empty());
    }
}
