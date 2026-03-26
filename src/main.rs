use bevy::prelude::*;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "Bangladesh RPG")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand, Debug)]
enum Mode {
    /// Host a local game (runs both client and server)
    Host,
    /// Run a dedicated server with no GUI
    Server,
    /// Connect to a remote server
    Client,
}

fn main() {
    // Basic argument parsing
    let cli = Cli::parse();
    
    let mut app = App::new();
    
    match cli.mode {
        Mode::Host => {
            println!("Starting Local Host (Client + Server)...");
            // Typical setup for both hosting the game state locally and viewing it
            app.add_plugins(DefaultPlugins);
            app.add_systems(Startup, host_setup);
        },
        Mode::Server => {
            println!("Starting Dedicated Server (No GUI)...");
            // MinimalPlugins allows running headless without windows/rendering
            app.add_plugins(MinimalPlugins);
            app.add_systems(Startup, server_setup);
        },
        Mode::Client => {
            println!("Starting Game Client...");
            // GUI client for connecting to a remote server
            app.add_plugins(DefaultPlugins);
            app.add_systems(Startup, client_setup);
        }
    }
    
    // Add common core systems here (e.g. shared logic, physics)
    // app.add_plugins(SharedGameLogicPlugin);
    
    app.run();
}

fn host_setup() {
    println!("Initializing host: setting up server bounds, spawning local player...");
    // Setup for running the server logic locally + presenting the GUI
}

fn server_setup() {
    println!("Initializing dedicated server: loading GIS map chunks, listening on socket...");
    // Headless logic: open network port, load terrain data into memory from GIS file
}

fn client_setup() {
    println!("Initializing client: resolving server address, setting up UI UI...");
    // Connect to network socket to sync state with server
}

