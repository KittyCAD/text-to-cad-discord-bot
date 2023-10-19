use serenity::{
    builder::CreateApplicationCommand, model::prelude::interaction::application_command::CommandDataOption,
};

pub fn run(options: &[CommandDataOption]) -> String {
    println!("design: {:?}", options);
    "design run".to_string()
}

pub fn register(command: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    command
        .name("design")
        .description("Generate a CAD model from a text prompt")
}
