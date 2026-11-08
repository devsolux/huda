//! Native helper. Explicit `init` bootstraps under the profile lease; `core` owns runtime writes.
//! All ordinary clients use private IPC rather than opening their own authority.
use clap::{Parser, Subcommand, ValueEnum};
use huda::answer::llm::{LocalHttpModel, LocalLlm, LocalModelConfig, UnavailableModel};
use huda::error::{require, Error, Result};
use huda::mcp::clients;
use huda::runtime::{auth, auth::OsSecrets, ipc::Client, Profile};
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
    about = huda::DESCRIPTION
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
    /// Recover or replace this profile's credentials. Works while no Core serves the profile.
    Credentials {
        #[command(subcommand)]
        action: CredentialsCommand,
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
    Stdio {
        /// Names the client in its configuration, so its row can show what was seen of it.
        #[arg(long)]
        client: Option<String>,
    },
    /// List AI clients and connect them. Changes only their own configuration files.
    Clients {
        #[command(subcommand)]
        action: ClientsCommand,
    },
}
#[derive(Subcommand)]
enum CredentialsCommand {
    /// Issue a new credential for each principal whose credential is missing from the OS secret
    /// store or no longer accepted. Credentials that still work are kept.
    Restore,
    /// Replace one principal's credential, for example after it may have leaked.
    Rotate { actor: String },
}
#[derive(Subcommand)]
enum ClientsCommand {
    /// Show each client's real state. Reads files; changes nothing and starts nothing.
    List,
    /// Show what connecting a client changes; with --yes, make that change and keep a backup.
    Connect {
        client: String,
        #[arg(long)]
        yes: bool,
    },
    /// Show what disconnecting a client changes; with --yes, remove only Huda's entry.
    Disconnect {
        client: String,
        #[arg(long)]
        yes: bool,
    },
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
fn client_named(name: &str) -> Result<clients::ClientId> {
    clients::ClientId::from_id(name)
        .ok_or_else(|| Error::invalid("Unknown client; huda mcp clients list shows every client"))
}
/// A second step for what changes another product's file: without --yes this only shows it.
fn confirmed(
    action: &str,
    preview: &clients::Preview,
    yes: bool,
    change: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    if !yes || !preview.automatic {
        let next = if preview.automatic {
            "Nothing was changed. Run this again with --yes to make this change."
        } else {
            "Huda does not change this client by itself here. Change it by hand."
        };
        return Ok(json!({"action": action, "applied": false, "preview": preview, "next": next}));
    }
    change()?;
    Ok(
        json!({"action": action, "applied": true, "preview": preview, "note": clients::PROTOCOL_NOTE}),
    )
}
/// Works on client configuration files only: this opens no database and starts no Core.
fn run_clients(action: ClientsCommand, profile: &Profile) -> Result<()> {
    let server = clients::Server::of_this_install(profile)?;
    let result = match action {
        ClientsCommand::List => json!({
            "note": clients::PROTOCOL_NOTE,
            "profile_initialized": profile.database().exists(),
            "clients": clients::status(&server, profile),
        }),
        ClientsCommand::Connect { client, yes } => {
            let client = client_named(&client)?;
            confirmed("connect", &clients::preview(client, &server)?, yes, || {
                clients::connect(client, &server)
            })?
        }
        ClientsCommand::Disconnect { client, yes } => {
            let client = client_named(&client)?;
            confirmed(
                "disconnect",
                &clients::preview(client, &server)?,
                yes,
                || clients::disconnect(client),
            )?
        }
    };
    print(&result)
}
/// Local recovery under the Core lease. Someone who lost a credential cannot authenticate, so the
/// trust is the OS account that owns the profile, as for `init`; no Core offers these operations.
fn run_credentials(action: CredentialsCommand, profile: &Profile) -> Result<()> {
    let mut store = huda::storage::Store::open(profile.clone()).map_err(|error| {
        if error.code == "CORE_UNAVAILABLE" {
            Error::new(
                "CORE_UNAVAILABLE",
                "A Huda Core is serving this profile, so its credentials cannot change now. A Core started on demand stops 5 minutes after its last request: close the AI clients that use Huda, wait, then try again",
            )
        } else {
            error
        }
    })?;
    let result = match action {
        CredentialsCommand::Restore => {
            let restored = auth::restore_access(&mut store, profile)?;
            json!({"restored": restored, "credentials": "os_secret_store"})
        }
        CredentialsCommand::Rotate { actor } => {
            auth::rotate_credential(&mut store, profile, &actor)?;
            json!({"rotated": actor, "credentials": "os_secret_store"})
        }
    };
    print(&result)
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
    let client = Client::new(profile.clone(), actor, !options.no_start);
    match options.command {
        Command::Init => {
            let mut store = huda::storage::Store::open(profile.clone())?;
            require(
                !store.has_principals()?,
                "VERSION_CONFLICT",
                "Profile is already initialized; credentials were not overwritten. If a credential is missing, run huda credentials restore",
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
                Some(std::thread::spawn(move || {
                    let result = server.serve(app, flag.clone());
                    // Without its listener the Core must not keep serving as if REST were up.
                    flag.store(true, Ordering::Release);
                    result
                }))
            } else {
                None
            };
            #[cfg(not(feature = "rest"))]
            require(
                rest_port.is_none(),
                "ADAPTER_UNAVAILABLE",
                "Build with --features rest before opting into a loopback listener",
            )?;
            let result = huda::runtime::ipc::serve_announced(
                app,
                profile,
                idle_seconds,
                shutdown.clone(),
                || {
                    eprintln!(
                        "Huda {} Core owner ready; no model downloads or source scans",
                        huda::VERSION
                    )
                },
            );
            shutdown.store(true, Ordering::Release);
            #[cfg(feature = "rest")]
            if let Some(http) = http {
                http.join()
                    .map_err(|_| Error::new("WORKER_FAILED", "REST worker failed"))??;
            }
            result
        }
        Command::Mcp {
            transport: McpCommand::Stdio { client: name },
        } => {
            let observer = name
                .as_deref()
                .and_then(|name| clients::Observer::new(&profile, name));
            huda::mcp::stdio(client, observer)
        }
        Command::Mcp {
            transport: McpCommand::Clients { action },
        } => run_clients(action, &profile),
        Command::Credentials { action } => run_credentials(action, &profile),
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
