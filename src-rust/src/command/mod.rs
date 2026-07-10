use std::collections::HashMap;

pub trait Command: Send {
    fn execute(&self) -> Result<(), anyhow::Error>;
    fn name(&self) -> &'static str;
}

pub struct CommandRegistry {
    commands: HashMap<String, Box<dyn Fn(&[String]) -> Result<Box<dyn Command>, anyhow::Error>>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            commands: HashMap::new(),
        };
        registry.register_builtins();
        registry
    }

    fn register_builtins(&mut self) {
        self.register("exec", |args| {
            Ok(Box::new(ExecCommand { command: args.join(" ") }))
        });
        self.register("restart", |_| {
            Ok(Box::new(RestartCommand))
        });
        self.register("quit", |_| {
            Ok(Box::new(QuitCommand))
        });
        self.register("reconfigure", |_| {
            Ok(Box::new(ReconfigureCommand))
        });
        self.register("reloadtheme", |_| {
            Ok(Box::new(ReconfigureCommand))
        });
    }

    pub fn register<F>(&mut self, name: &str, factory: F)
    where
        F: Fn(&[String]) -> Result<Box<dyn Command>, anyhow::Error> + 'static,
    {
        self.commands.insert(name.to_string(), Box::new(factory));
    }

    pub fn create(&self, name: &str, args: &[String]) -> Result<Box<dyn Command>, anyhow::Error> {
        if let Some(factory) = self.commands.get(name) {
            factory(args)
        } else {
            Err(anyhow::anyhow!("Unknown command: {}", name))
        }
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ExecCommand {
    pub command: String,
}

impl Command for ExecCommand {
    fn execute(&self) -> Result<(), anyhow::Error> {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .spawn()?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "exec"
    }
}

pub struct RestartCommand;

impl Command for RestartCommand {
    fn execute(&self) -> Result<(), anyhow::Error> {
        log::info!("Restart requested");
        Ok(())
    }

    fn name(&self) -> &'static str {
        "restart"
    }
}

pub struct QuitCommand;

impl Command for QuitCommand {
    fn execute(&self) -> Result<(), anyhow::Error> {
        log::info!("Quit requested");
        std::process::exit(0);
    }

    fn name(&self) -> &'static str {
        "quit"
    }
}

pub struct ReconfigureCommand;

impl Command for ReconfigureCommand {
    fn execute(&self) -> Result<(), anyhow::Error> {
        log::info!("Reconfigure requested");
        Ok(())
    }

    fn name(&self) -> &'static str {
        "reconfigure"
    }
}
