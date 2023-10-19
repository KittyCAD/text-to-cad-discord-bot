//! A discord bot for interacting with the KittyCAD text-to-cad API.

#![deny(missing_docs)]

#[macro_use]
mod enclose;
mod server;
#[cfg(test)]
mod tests;

use std::{collections::HashSet, sync::Arc};

use anyhow::{bail, Result};
use clap::Parser;
use serenity::{
    async_trait,
    client::bridge::gateway::{ShardId, ShardManager},
    framework::standard::{
        help_commands,
        macros::{check, command, group, help, hook},
        Args, CommandGroup, CommandOptions, CommandResult, DispatchError, HelpOptions, Reason, StandardFramework,
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
        slog::Logger::root(drain, slog::slog_o!())
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
    pub fn create_logger(&self) -> slog::Logger {
        if self.json {
            let drain = slog_json::Json::default(std::io::stderr()).fuse();
            self.async_root_logger(drain)
        } else {
            let decorator = slog_term::TermDecorator::new().build();
            let drain = slog_term::FullFormat::new(decorator).build().fuse();
            self.async_root_logger(drain)
        }
    }

    fn async_root_logger<T>(&self, drain: T) -> slog::Logger
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
        slog::Logger::root(async_drain, slog::o!())
    }
}

/// A subcommand for our cli.
#[derive(Parser, Debug, Clone)]
pub enum SubCommand {
    /// Run the server.
    Server(Server),
}

/// A subcommand for running the server.
#[derive(Parser, Clone, Debug)]
pub struct Server {
    /// IP address and port that the server should listen
    #[clap(short, long, default_value = "0.0.0.0:8080")]
    pub address: String,

    /// The discord bot token to use.
    #[clap(short, long, env = "DISCORD_TOKEN")]
    pub discord_token: String,

    /// The discord client ID to use.
    #[clap(short, long, env = "DISCORD_CLIENT_ID")]
    pub discord_client_id: String,

    /// The discord client secret to use.
    #[clap(short, long, env = "DISCORD_CLIENT_SECRET")]
    pub discord_client_secret: String,

    /// The discord redirect URI to use.
    #[clap(short, long, env = "DISCORD_REDIRECT_URI")]
    pub discord_redirect_uri: String,
}

// A container type is created for inserting into the Client's `data`, which
// allows for data to be accessible across all events and framework commands, or
// anywhere else that has a copy of the `data` Arc.
struct ShardManagerContainer;

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }
}

#[group]
#[commands(about, design, latency, ping)]
struct General;

#[group]
#[owners_only]
// Limit all commands to be guild-restricted.
#[only_in(guilds)]
// Summary only appears when listing multiple groups.
#[summary = "Commands for server owners"]
struct Owner;

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
async fn before(_ctx: &Context, msg: &Message, command_name: &str) -> bool {
    println!("Got command '{}' by user '{}'", command_name, msg.author.name);

    true // if `before` returns false, command processing doesn't happen.
}

#[hook]
async fn after(_ctx: &Context, _msg: &Message, command_name: &str, command_result: CommandResult) {
    match command_result {
        Ok(()) => println!("Processed command '{}'", command_name),
        Err(why) => println!("Command '{}' returned error {:?}", command_name, why),
    }
}

#[hook]
async fn unknown_command(_ctx: &Context, _msg: &Message, unknown_command_name: &str) {
    println!("Could not find command named '{}'", unknown_command_name);
}

#[hook]
async fn normal_message(_ctx: &Context, msg: &Message) {
    println!("Message is not a command '{}'", msg.content);
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
                .channel_id
                .say(&ctx.http, &format!("Try this again in {} seconds.", info.as_secs()))
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
            }

            // start listening for events by starting a single shard
            client.start().await?;

            // If we get here, the client has disconnected cleanly.
            // So we should abort our handle for our server.
            handle.abort();
        }
    }

    Ok(())
}

// A function which acts as a "check", to determine whether to call a command.
//
// In this case, this command checks to ensure you are the owner of the message
// in order for the command to be executed. If the check fails, the command is
// not called.
#[check]
#[name = "Owner"]
async fn owner_check(_: &Context, msg: &Message, _: &mut Args, _: &CommandOptions) -> Result<(), Reason> {
    // Replace 7 with your ID to make this check pass.
    //
    // 1. If you want to pass a reason alongside failure you can do:
    // `Reason::User("Lacked admin permission.".to_string())`,
    //
    // 2. If you want to mark it as something you want to log only:
    // `Reason::Log("User lacked admin permission.".to_string())`,
    //
    // 3. If the check's failure origin is unknown you can mark it as such:
    // `Reason::Unknown`
    //
    // 4. If you want log for your system and for the user, use:
    // `Reason::UserAndLog { user, log }`
    if msg.author.id != 7 {
        return Err(Reason::User("Lacked owner permission".to_string()));
    }

    Ok(())
}

#[command]
async fn about(ctx: &Context, msg: &Message) -> CommandResult {
    msg.channel_id
        .say(
            &ctx.http,
            "A discord bot to play with the KittyCAD Text to CAD API. : )",
        )
        .await?;

    Ok(())
}

#[command]
// Limit command usage to guilds.
#[only_in(guilds)]
#[checks(Owner)]
async fn latency(ctx: &Context, msg: &Message) -> CommandResult {
    // The shard manager is an interface for mutating, stopping, restarting, and
    // retrieving information about shards.
    let data = ctx.data.read().await;

    let shard_manager = match data.get::<ShardManagerContainer>() {
        Some(v) => v,
        None => {
            msg.reply(ctx, "There was a problem getting the shard manager").await?;

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
            msg.reply(ctx, "No shard found").await?;

            return Ok(());
        }
    };

    msg.reply(ctx, &format!("The shard latency is {:?}", runner.latency))
        .await?;

    Ok(())
}

#[command]
// Limit command usage to guilds.
#[only_in(guilds)]
#[checks(Owner)]
async fn ping(ctx: &Context, msg: &Message) -> CommandResult {
    msg.channel_id.say(&ctx.http, "Pong! : )").await?;

    Ok(())
}

#[command]
#[description = "Generate a CAD model from a text prompt."]
async fn design(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    match args.single_quoted::<String>() {
        Ok(x) => {
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

            msg.channel_id.say(&ctx.http, &content).await?;

            return Ok(());
        }
        Err(_) => {
            msg.reply(ctx, "An argument is required to run this command.").await?;
            return Ok(());
        }
    };
}
