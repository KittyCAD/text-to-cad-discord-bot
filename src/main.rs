//! A discord bot for interacting with the KittyCAD text-to-cad API.

#![deny(missing_docs)]

mod image;
#[macro_use]
mod enclose;
mod server;
#[cfg(test)]
mod tests;

use std::{borrow::Borrow, collections::HashSet, sync::Arc};

use anyhow::{bail, Result};
use clap::Parser;
use serenity::{
    async_trait,
    client::bridge::gateway::{ShardId, ShardManager},
    framework::standard::{
        help_commands,
        macros::{command, group, help, hook},
        Args, CommandGroup, CommandResult, DispatchError, HelpOptions, StandardFramework,
    },
    http::Http,
    model::{
        channel::Message,
        gateway::{GatewayIntents, Ready},
        id::UserId,
    },
    prelude::*,
    utils::{content_safe, ContentSafeOptions},
};
use slog::Drain;
use tracing_subscriber::{prelude::*, Layer};

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

// A container type is created for inserting into the Client's `data`, which
// allows for data to be accessible across all events and framework commands, or
// anywhere else that has a copy of the `data` Arc.
struct ShardManagerContainer;

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}

struct KittycadApi;

impl TypeMapKey for KittycadApi {
    type Value = kittycad::Client;
}

struct Logger;

impl TypeMapKey for Logger {
    type Value = slog::Logger;
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        let data = ctx.data.read().await;
        let logger = data.get::<Logger>().unwrap().clone();

        slog::info!(logger, "{} is connected!", ready.user.name);
    }
}

#[group]
#[commands(about)]
struct General;

#[group]
#[owners_only]
// Limit all commands to be guild-restricted.
#[only_in(guilds)]
#[commands(design, latency, ping)]
// Summary only appears when listing multiple groups.
#[summary = "Commands for server owners"]
struct Owner;

#[group]
#[allowed_roles("kittycad staff")]
// Limit all commands to be guild-restricted.
#[only_in(guilds)]
#[commands(design, latency, ping)]
// Summary only appears when listing multiple groups.
#[summary = "Commands for KittyCAD staff"]
struct KittyCadStaff;

// The framework provides two built-in help commands for you to use.
// But you can also make your own customized help command that forwards
// to the behaviour of either of them.
#[help]
async fn bot_help(
    context: &Context,
    msg: &Message,
    args: Args,
    help_options: &'static HelpOptions,
    groups: &[&'static CommandGroup],
    owners: HashSet<UserId>,
) -> CommandResult {
    let _ = help_commands::with_embeds(context, msg, args, help_options, groups, owners).await;
    Ok(())
}

#[hook]
async fn before(ctx: &Context, msg: &Message, command_name: &str) -> bool {
    let data = ctx.data.read().await;
    let logger = data.get::<Logger>().unwrap().clone();

    slog::info!(logger, "Got command '{}' by user '{}'", command_name, msg.author.name);

    true // if `before` returns false, command processing doesn't happen.
}

#[hook]
async fn after(ctx: &Context, _msg: &Message, command_name: &str, command_result: CommandResult) {
    let data = ctx.data.read().await;
    let logger = data.get::<Logger>().unwrap().clone();

    match command_result {
        Ok(()) => slog::info!(logger, "Processed command '{}'", command_name),
        Err(why) => slog::info!(logger, "Command '{}' returned error {:?}", command_name, why),
    }
}

#[hook]
async fn unknown_command(ctx: &Context, _msg: &Message, unknown_command_name: &str) {
    let data = ctx.data.read().await;
    let logger = data.get::<Logger>().unwrap().clone();

    slog::info!(logger, "Could not find command named '{}'", unknown_command_name);
}

#[hook]
async fn normal_message(ctx: &Context, msg: &Message) {
    let data = ctx.data.read().await;
    let logger = data.get::<Logger>().unwrap().clone();

    slog::debug!(logger, "Message is not a command '{}'", msg.content);
}

#[hook]
async fn delay_action(ctx: &Context, msg: &Message) {
    // You may want to handle a Discord rate limit if this fails.
    let _ = msg.react(ctx, '⏱').await;
}

#[hook]
async fn dispatch_error(ctx: &Context, msg: &Message, error: DispatchError, _command_name: &str) {
    if let DispatchError::Ratelimited(info) = error {
        // We notify them only once.
        if info.is_first_try {
            let _ = msg
                .reply(&ctx.http, &format!("Try this again in {} seconds.", info.as_secs()))
                .await;
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

            // Login with a bot token from the environment
            let http = Http::new(&s.discord_token);

            // We will fetch your bot's owners and id
            let info = http.get_current_application_info().await?;
            let mut owners = HashSet::new();
            if let Some(team) = info.team {
                owners.insert(team.owner_user_id);
            } else {
                owners.insert(info.owner.id);
            }
            let bot_id = http.get_current_user().await?;

            // Set up the framework.
            let framework = StandardFramework::new()
                .configure(|c| {
                    c.with_whitespace(true)
                        .on_mention(Some(bot_id.id))
                        .prefix("~")
                        // In this case, if "," would be first, a message would never
                        // be delimited at ", ", forcing you to trim your arguments if you
                        // want to avoid whitespaces at the start of each.
                        .delimiters(vec![", ", ","])
                        // Sets the bot's owners. These will be used for commands that
                        // are owners only.
                        .owners(owners)
                })
                // Set a function to be called prior to each command execution. This
                // provides the context of the command, the message that was received,
                // and the full name of the command that will be called.
                //
                // Avoid using this to determine whether a specific command should be
                // executed. Instead, prefer using the `#[check]` macro which
                // gives you this functionality.
                //
                // **Note**: Async closures are unstable, you may use them in your
                // application if you are fine using nightly Rust.
                // If not, we need to provide the function identifiers to the
                // hook-functions (before, after, normal, ...).
                .before(before)
                // Similar to `before`, except will be called directly _after_
                // command execution.
                .after(after)
                // Set a function that's called whenever an attempted command-call's
                // command could not be found.
                .unrecognised_command(unknown_command)
                // Set a function that's called whenever a message is not a command.
                .normal_message(normal_message)
                // Set a function that's called whenever a command's execution didn't complete for one
                // reason or another. For example, when a user has exceeded a rate-limit or a command
                // can only be performed by the bot owner.
                .on_dispatch_error(dispatch_error)
                // The `#[group]` macro generates `static` instances of the options set for the group.
                // They're made in the pattern: `#name_GROUP` for the group instance and `#name_GROUP_OPTIONS`.
                // #name is turned all uppercase
                .help(&BOT_HELP)
                .group(&GENERAL_GROUP)
                .group(&KITTYCADSTAFF_GROUP)
                .group(&OWNER_GROUP);

            // For this example to run properly, the "Presence Intent" and "Server Members Intent"
            // options need to be enabled.
            // These are needed so the `required_permissions` macro works on the commands that need to
            // use it.
            // You will need to enable these 2 options on the bot application, and possibly wait up to 5
            // minutes.
            let intents = GatewayIntents::all();
            let mut client = Client::builder(&s.discord_token, intents)
                .event_handler(Handler)
                .framework(framework)
                .await?;

            {
                let mut data = client.data.write().await;
                data.insert::<ShardManagerContainer>(Arc::clone(&client.shard_manager));
                data.insert::<KittycadApi>(kittycad::Client::new(&s.kittycad_api_token));
                data.insert::<Logger>(crate::LOGGER.clone());
            }

            // start listening for events by starting a single shard
            client.start().await?;

            // If we get here, the client has disconnected cleanly.
            // So we should abort our handle for our server.
            handle.abort();
        }

        SubCommand::ConvertImage(c) => {
            let logger = opts.create_logger("convert-image");
            let image_bytes = crate::image::model_to_image(&logger, &c.gltf_path).await?;

            tokio::fs::write(&c.image_path, image_bytes).await?;
        }
        SubCommand::TextToCad(t) => {
            let logger = opts.create_logger("text-to-cad");

            let mut users_client = kittycad::Client::new(&t.kittycad_api_token);
            users_client.set_base_url("https://api.dev.kittycad.io");

            let (image_file, mut model) = get_image_bytes_for_prompt(&logger, &users_client, &t.prompt)
                .await
                .unwrap();

            // Clear the outputs so we don't print them.
            model.outputs = Default::default();
            slog::info!(logger, "Model: {:?}", model);
            slog::info!(logger, "Image file: {:?}", image_file);
        }
    }

    Ok(())
}

#[command]
#[num_args(0)]
async fn about(ctx: &Context, msg: &Message) -> CommandResult {
    msg.reply(&ctx.http, "A discord bot to play with the KittyCAD Text to CAD API. 🤪")
        .await?;

    Ok(())
}

#[command]
// Limit command usage to guilds.
#[only_in(guilds)]
#[num_args(0)]
async fn latency(ctx: &Context, msg: &Message) -> CommandResult {
    // The shard manager is an interface for mutating, stopping, restarting, and
    // retrieving information about shards.
    let data = ctx.data.read().await;

    let shard_manager = match data.get::<ShardManagerContainer>() {
        Some(v) => v,
        None => {
            msg.reply(ctx, "‼️  There was a problem getting the shard manager")
                .await?;

            return Ok(());
        }
    };

    let manager = shard_manager.lock().await;
    let runners = manager.runners.lock().await;

    // Shards are backed by a "shard runner" responsible for processing events
    // over the shard, so we'll get the information about the shard runner for
    // the shard this command was sent over.
    let runner = match runners.get(&ShardId(ctx.shard_id)) {
        Some(runner) => runner,
        None => {
            msg.reply(ctx, "‼️  No shard found").await?;

            return Ok(());
        }
    };

    msg.reply(ctx, &format!("💨 The shard latency is {:?}", runner.latency))
        .await?;

    Ok(())
}

#[command]
// Limit command usage to guilds.
#[only_in(guilds)]
#[num_args(0)]
async fn ping(ctx: &Context, msg: &Message) -> CommandResult {
    msg.reply(&ctx.http, "🏓").await?;

    Ok(())
}

#[command]
#[description = "Generate a CAD model from a text prompt."]
#[example = "design a 2x4 lego"]
async fn design(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let data = ctx.data.read().await;
    let logger = data.get::<Logger>().ok_or(anyhow::anyhow!("Logger not found"))?;

    match args.single_quoted::<String>() {
        Ok(x) => {
            // React to the message that we are working on it.
            // We want to use unicode eyes.
            msg.react(ctx, '👀').await?;

            let settings = if let Some(guild_id) = msg.guild_id {
                // By default roles, users, and channel mentions are cleaned.
                ContentSafeOptions::default()
                    // We do not want to clean channel mentions as they
                    // do not ping users.
                    .clean_channel(false)
                    // If it's a guild channel, we want mentioned users to be displayed
                    // as their display name.
                    .display_as_member_from(guild_id)
            } else {
                ContentSafeOptions::default().clean_channel(false).clean_role(false)
            };

            let content = content_safe(&ctx.cache, x, &settings, &msg.mentions);
            // Some times users say `design me` trim the `me ` to not confuse the model.
            let cleaned = content.trim_start_matches("me ");

            if let Err(err) = run_text_to_cad_prompt(ctx, msg, cleaned).await {
                // If the error was from the API, let's handle it better for each type of error.
                let e = match err.downcast_ref::<kittycad::types::error::Error>() {
                    Some(kerr) => {
                        if let kittycad::types::error::Error::Server { body, status } = kerr {
                            // Get the body of the response.
                            let error: kittycad::types::Error =
                                serde_json::from_str(body).unwrap_or(kittycad::types::Error {
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
                    msg.react(ctx, '🔒').await?;
                    // Send the message as a DM so we don't spam the channel.
                    msg.author.direct_message(ctx, |m| m.content(message)).await?;

                    // Return early so we don't send the message to the channel.
                    return Ok(());
                }

                msg.reply(ctx, &message).await?;
                msg.react(ctx, '🤮').await?;
            }

            return Ok(());
        }
        Err(_) => {
            msg.reply(ctx, "🫠 An argument is required to run this command.")
                .await?;
            return Ok(());
        }
    };
}

async fn run_text_to_cad_prompt(ctx: &Context, msg: &Message, prompt: &str) -> Result<()> {
    let data = ctx.data.read().await;
    let logger = data.get::<Logger>().ok_or(anyhow::anyhow!("Logger not found"))?;
    let kittycad_client = data
        .get::<KittycadApi>()
        .ok_or(anyhow::anyhow!("Kittycad client not found"))?;

    // Get a token for the user based on their discord id.
    let kittycad_token = kittycad_client
        .meta()
        .internal_get_api_token_for_discord_user(&msg.author.id.0.to_string())
        .await?;
    // Now create a new kittycad client with the user's token.
    let users_client = kittycad::Client::new(kittycad_token.token);

    // This is CPU bound so let's force it on another thread.
    let (image_path, model) = get_image_bytes_for_prompt(logger, &users_client, prompt).await?;

    // Show that we are done working on it.
    msg.react(ctx, '🥳').await?;

    let feedback_time_seconds = 120;
    let our_msg = msg
        .channel_id
        .send_message(&ctx.http, |m| {
            let message = m
                .content(msg.author.mention())
                .embed(|e| {
                    e.title(prompt)
                        .image(&format!(
                            "attachment://{}",
                            image_path.file_name().unwrap().to_string_lossy()
                        ))
                        // Thumbs up or down emoji.
                        .footer(|f| {
                            f.text(&format!(
                                r#"React with a 👍 or 👎 to this message to give feedback.
Feedback must be left within the next {} seconds.
After that, the original prompter can login to view their model or give feedback at:
https://https://text-to-cad.kittycad.io/view/{}"#,
                                feedback_time_seconds, model.id
                            ))
                        })
                        // Add a timestamp for the current time
                        // This also accepts a rfc3339 Timestamp
                        .timestamp(serenity::model::Timestamp::now())
                })
                .add_file(&image_path);
            slog::info!(logger, "Sending message: {:?}", message);
            message
        })
        .await?;

    // Remove the file.
    tokio::fs::remove_file(&image_path).await?;

    // Wait for feedback to the model based on emoji reaction.
    let reaction = our_msg
        .await_reaction(ctx)
        .author_id(msg.author.id)
        .added(true)
        .timeout(std::time::Duration::from_secs(feedback_time_seconds))
        .filter(|reaction| {
            reaction.emoji == serenity::model::channel::ReactionType::Unicode("👍".to_string())
                || reaction.emoji == serenity::model::channel::ReactionType::Unicode("👎".to_string())
        })
        .await;

    if let Some(feedback) = reaction {
        if let serenity::collector::reaction_collector::ReactionAction::Added(added) = feedback.borrow() {
            let reaction = if added.emoji == serenity::model::channel::ReactionType::Unicode("👍".to_string()) {
                Some(kittycad::types::AiFeedback::ThumbsUp)
            } else if added.emoji == serenity::model::channel::ReactionType::Unicode("👎".to_string()) {
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
            }
        }
    }
    // TODO: add export button.

    Ok(())
}

async fn get_image_bytes_for_prompt(
    logger: &slog::Logger,
    users_client: &kittycad::Client,
    prompt: &str,
) -> Result<(std::path::PathBuf, kittycad::types::TextToCad)> {
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
        && start.elapsed().as_secs() < 60 * 10
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
            anyhow::bail!("Your prompt returned an error: {}", error);
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

    // This is CPU bound so let's force it on another thread.
    let image_path = gltf_to_image(logger, &gltf_bytes).await?;

    Ok((image_path, model))
}

/// Re-execute ourselves to convert the gltf file to an image.
// We do this because the graphics lib we are using forces you to use the main thread.
async fn gltf_to_image(logger: &slog::Logger, contents: &[u8]) -> Result<std::path::PathBuf> {
    // Create a temp file.
    let temp_dir = std::env::temp_dir();
    let gltf_path = temp_dir.join(format!("{}.gltf", uuid::Uuid::new_v4()));
    let image_path = temp_dir.join(format!("{}.png", uuid::Uuid::new_v4()));

    // Write the gltf bytes to the file.
    tokio::fs::write(&gltf_path, contents).await?;

    let mut command = tokio::process::Command::new(path_to_self()?);
    let mut args = vec![
        "convert-image".to_string(),
        "--gltf-path".to_string(),
        gltf_path.to_string_lossy().to_string(),
        "--image-path".to_string(),
        image_path.to_string_lossy().to_string(),
    ];
    if cfg!(target_os = "linux") {
        // We need to run with a virtual x server.
        command = tokio::process::Command::new("xvfb-run");
        // Push an arg to the front of args.
        args.insert(0, "-a".to_string());
        args.insert(1, path_to_self()?.to_string_lossy().to_string());
    }
    // Re-execute ourselves to convert the gltf file to an image.
    let output = command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        slog::warn!(logger, "Convert image failed: {:?}", output);
        anyhow::bail!("Convert image failed: {:?}", output);
    }

    slog::info!(logger, "Convert image output: {:?}", output);

    // Remove the gltf file.
    tokio::fs::remove_file(&gltf_path).await?;

    // Return the image path.
    Ok(image_path)
}

#[cfg(test)]
fn path_to_self() -> Result<std::path::PathBuf> {
    let path = std::env::current_dir()?
        .join("target")
        .join("debug")
        .join("text-to-cad-discord-bot");
    Ok(path)
}

#[cfg(not(test))]
fn path_to_self() -> Result<std::path::PathBuf> {
    Ok(std::env::current_exe()?)
}

#[cfg(test)]
mod test {
    use slog::Drain;

    use crate::get_image_bytes_for_prompt;

    #[tokio::test]
    async fn test_get_image_from_prompt() {
        let logger = {
            let decorator = slog_term::PlainDecorator::new(slog_term::TestStdoutWriter);
            let drain = std::sync::Mutex::new(slog_term::FullFormat::new(decorator).build()).fuse();

            slog::Logger::root(drain, slog::o!())
        };
        let mut kittycad_client = kittycad::Client::new_from_env();
        kittycad_client.set_base_url("https://api.dev.kittycad.io");

        let (image_file, _model) = get_image_bytes_for_prompt(&logger, &kittycad_client, "a 2x4 lego")
            .await
            .unwrap();

        // Make sure the file exists.
        assert!(image_file.exists());

        // Read the contents of the file.
        let image_bytes = std::fs::read(&image_file).unwrap();
        assert!(!image_bytes.is_empty());

        std::fs::remove_file(&image_file).unwrap();
    }
}
