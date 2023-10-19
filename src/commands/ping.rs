use serenity::{
    builder::CreateApplicationCommand, model::prelude::interaction::application_command::CommandDataOption,
};

pub fn run(_options: &[CommandDataOption]) -> String {
    "Hey, I'm alive!".to_string()
}

pub fn register(command: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    command.name("ping").description("A ping command")
}
