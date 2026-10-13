//! Native helper. Explicit `init` bootstraps under the profile lease; `core` owns runtime writes.
//! All ordinary clients use private IPC rather than opening their own authority.
use clap::{Parser, Subcommand, ValueEnum};
use huda::answer::llm::{LocalHttpModel, LocalLlm, LocalModelConfig, UnavailableModel};
use huda::error::{require, Result};
use huda::runtime::{auth::OsSecrets, ipc::Client, Profile};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Parser)]
#[command(
    name = "huda",
    version,
    about = "Local, relation-first knowledge space"
)]
struct Options {
    #[arg(long, global = true)]
    profile: Option<PathBuf>,
    #[arg(long, global = true, default_value = "owner")]
    actor: String,
    #[arg(long, global = true)]
    no_start: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Explicitly create an empty offline profile and credentials in the OS secret store.
    Init,
    /// Run the single Core owner. Does not install a service or open a network port by default.
    Core {
        #[arg(long, default_value_t = 300)]
        idle_seconds: u64,
        /// Explicit local model configuration. Does not download or start a model.
        #[arg(long)]
        model_config: Option<PathBuf>,
        /// Explicit opt-in REST listener on 127.0.0.1; requires the rest build feature.
        #[arg(long)]
        rest_port: Option<u16>,
    },
    Mcp {
        #[command(subcommand)]
        transport: McpCommand,
    },
    Status,
    Ask {
        question: String,
        #[arg(long, default_value = "en")]
        language: String,
        #[arg(long, value_enum, default_value_t = Expansion::Off)]
        expansion: Expansion,
    },
    /// Invoke a documented common application handler. JSON comes from a file or bounded stdin.
    Call {
        operation: String,
        #[arg(long)]
        input: Option<PathBuf>,
    },
    /// Export OpenAPI 3.1.2 directly from handler DTOs. Never overwrites an existing file.
    Contracts {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compute a canonical entity ID without opening a database or publishing anything.
    EntityId {
        #[arg(long)]
        input: Option<PathBuf>,
    },
}
#[derive(Subcommand)]
enum McpCommand {
    Stdio,
}
#[derive(Clone, Copy, ValueEnum)]
enum Expansion {
    Off,
    Suggest,
    Auto,
}

fn main() -> std::process::ExitCode {
    match run(Options::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // No payloads, local paths, tokens, raw SQL or model output in diagnostic logs.
            eprintln!("{}", json!({"error": error}));
            std::process::ExitCode::FAILURE
        }
    }
}
fn read_json(path: Option<&Path>) -> Result<Value> {
    let reader: Box<dyn Read> = match path {
        Some(path) if path != Path::new("-") => Box::new(std::fs::File::open(path)?),
        _ => Box::new(std::io::stdin()),
    };
    let mut bytes = Vec::new();
    reader
        .take(huda::MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    require(
        bytes.len() <= huda::MAX_MESSAGE_BYTES,
        "LIMIT_EXCEEDED",
        "CLI JSON input exceeds the message limit",
    )?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}
fn print(value: &Value) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
fn run(options: Options) -> Result<()> {
    // These operations are deterministic file/stdout work, not Core startup.
    match &options.command {
        Command::Contracts { output } => {
            let contract = huda::api::contracts::openapi();
            if let Some(path) = output {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                serde_json::to_writer_pretty(&mut file, &contract)?;
                file.write_all(b"\n")?;
                file.sync_all()?;
                return Ok(());
            }
            return print(&contract);
        }
        Command::EntityId { input } => {
            let entity: huda::relation::EntityInput =
                serde_json::from_value(read_json(input.as_deref())?)?;
            return print(&json!({"entity_id": entity.id()?}));
        }
        _ => {}
    }
    let profile = match options.profile {
        Some(path) => Profile::new(path)?,
        None => Profile::default_location()?,
    };
    let actor = if matches!(&options.command, Command::Mcp { .. }) && options.actor == "owner" {
        "mcp".to_owned()
    } else {
        options.actor
    };
    let client = Client {
        profile: profile.clone(),
        actor,
        start_on_demand: !options.no_start,
    };
    match options.command {
        Command::Init => {
            let mut store = huda::storage::Store::open(profile.clone())?;
            require(
                !store.has_principals()?,
                "VERSION_CONFLICT",
                "Profile is already initialized; credentials were not overwritten",
            )?;
            let owner = OsSecrets::create(&profile, "owner")?;
            let mcp = OsSecrets::create(&profile, "mcp")?;
            store.bootstrap_credentials(&owner, &mcp)?;
            print(
                &json!({"initialized": true, "version": huda::VERSION, "offline": true, "credentials": "os_secret_store"}),
            )
        }
        Command::Core {
            idle_seconds,
            model_config,
            rest_port,
        } => {
            require(
                idle_seconds <= 86_400,
                "INVALID_INPUT",
                "Idle lifetime exceeds one day; zero selects explicit foreground lifetime",
            )?;
            let store = huda::storage::Store::open(profile.clone())?;
            require(
                store.has_principals()?,
                "AUTH_REQUIRED",
                "Run huda init explicitly to initialize this offline profile",
            )?;
            let model: Arc<dyn LocalLlm> = match model_config {
                Some(path) => {
                    let config: LocalModelConfig = serde_json::from_value(read_json(Some(&path))?)?;
                    Arc::new(LocalHttpModel::new(config)?)
                }
                None => Arc::new(UnavailableModel),
            };
            let app = huda::api::App::new(store, model);
            let shutdown = Arc::new(AtomicBool::new(false));
            #[cfg(feature = "rest")]
            let http = if let Some(port) = rest_port {
                let server = huda::api::rest::RestServer::bind(port)?;
                let app = app.clone();
                let flag = shutdown.clone();
                Some(std::thread::spawn(move || server.serve(app, flag)))
            } else {
                None
            };
            #[cfg(not(feature = "rest"))]
            require(
                rest_port.is_none(),
                "ADAPTER_UNAVAILABLE",
                "Build with --features rest before opting into a loopback listener",
            )?;
            eprintln!(
                "Huda {} Core owner ready; no model downloads or source scans",
                huda::VERSION
            );
            let result = huda::runtime::ipc::serve(app, profile, idle_seconds, shutdown.clone());
            shutdown.store(true, Ordering::Release);
            #[cfg(feature = "rest")]
            if let Some(http) = http {
                http.join().map_err(|_| {
                    huda::error::Error::new("WORKER_FAILED", "REST worker failed")
                })??;
            }
            result
        }
        Command::Mcp {
            transport: McpCommand::Stdio,
        } => huda::mcp::stdio(client),
        Command::Status => print(&client.invoke("status", json!({}))?),
        Command::Ask {
            question,
            language,
            expansion,
        } => {
            let mode = match expansion {
                Expansion::Off => "off",
                Expansion::Suggest => "suggest",
                Expansion::Auto => "auto",
            };
            print(&client.invoke(
                "ask_huda",
                json!({"question": question, "language": language, "expansion_mode": mode}),
            )?)
        }
        Command::Call { operation, input } => {
            print(&client.invoke(&operation, read_json(input.as_deref())?)?)
        }
        Command::Contracts { .. } | Command::EntityId { .. } => {
            unreachable!("Handled before profile creation")
        }
    }
}
