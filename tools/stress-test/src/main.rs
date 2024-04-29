use std::cmp;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap::{Args, Parser, Subcommand};

use ockam::abac::tokio::runtime::Runtime;
use ockam::compat::tokio;
use ockam::compat::tokio::task::JoinSet;
use ockam::{Context, NodeBuilder, TcpListenerOptions, TcpTransport};
use ockam_api::nodes::service::{NodeManagerGeneralOptions, NodeManagerTransportOptions};
use ockam_api::nodes::{InMemoryNode, NodeManagerWorker, NODEMANAGER_ADDR};
use ockam_api::CliState;
use ockam_core::Address;

use crate::config::Config;
use crate::portal_simulator::PortalStats;

mod config;
mod portal_simulator;

#[derive(Debug, Args, Clone)]
struct RunCommand {
    config: PathBuf,
    #[arg(long)]
    log: bool,
}

#[derive(Debug, Args, Clone)]
struct ValidateCommand {
    config: PathBuf,
}

#[derive(Subcommand, Debug, Clone)]
enum Action {
    /// Run the stress test
    Run(RunCommand),
    /// Validate the configuration file
    Validate(ValidateCommand),
    /// Generate sample configuration files
    Generate,
}

#[derive(Debug, Parser, Clone)]
#[command(name = "stress-test")]
struct Main {
    /// Action to perform
    #[command(subcommand)]
    action: Action,
}

fn main() {
    let main: Main = Main::parse();
    match main.action {
        Action::Run(cmd) => run(cmd),
        Action::Validate(cmd) => validate(cmd),
        Action::Generate => generate(),
    }
}

fn generate() {
    println!("{}", Config::sample_configs());
}

fn validate(cmd: ValidateCommand) {
    match Config::parse(&cmd.config) {
        Ok(_config) => {
            println!("configuration file is valid");
        }
        Err(err) => {
            eprintln!("configuration file is invalid: {:?}", err.message());
            std::process::exit(1);
        }
    }
}

fn run(cmd: RunCommand) {
    let config = match Config::parse(&cmd.config) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("configuration file is invalid: {:?}", err.message());
            std::process::exit(1);
        }
    };

    let state = Arc::new(State::new(config, cmd.log));
    let runtime = Runtime::new().unwrap();

    {
        let state = state.clone();
        runtime.spawn(async move {
            loop {
                state.execute_delta().await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    }

    runtime.block_on(async {
        loop {
            state.measure_speed();
            state.print_summary();
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

struct Relay {
    failures_detected: u32,
    usages: u32,
}

const NODE_NAME: &str = "stress-tester";

struct State {
    relay_creation_failed: AtomicU32,
    portal_creation_failed: AtomicU32,
    portals: Arc<Mutex<HashMap<String, PortalStats>>>,
    relays: Arc<Mutex<HashMap<String, Relay>>>,
    begin: Instant,
    config: Config,
    node: Arc<InMemoryNode>,
    context: Arc<Context>,
    speed_stats: Arc<Mutex<Vec<(u64, u64)>>>,
    previous_bytes_sent: AtomicU64,
    previous_bytes_received: AtomicU64,
}

impl State {
    fn new(config: Config, log: bool) -> Self {
        let cli_state = CliState::with_default_dir().expect("cannot create cli state");
        let rt = Arc::new(Runtime::new().expect("cannot create a tokio runtime"));
        let builder = if log {
            NodeBuilder::new()
        } else {
            NodeBuilder::new().no_logging()
        };
        let (context, mut executor) = builder.with_runtime(rt.clone()).build();
        let context = Arc::new(context);

        // start the router, it is needed for the node manager creation
        rt.spawn(async move {
            executor
                .start_router()
                .await
                .expect("cannot start executor")
        });

        let runtime = context.runtime().clone();
        let node_manager = runtime
            .block_on(Self::make_node_manager(context.clone(), &cli_state))
            .expect("cannot create node manager");

        Self {
            relay_creation_failed: Default::default(),
            portal_creation_failed: Default::default(),
            portals: Default::default(),
            relays: Default::default(),
            speed_stats: Default::default(),
            previous_bytes_received: Default::default(),
            previous_bytes_sent: Default::default(),
            begin: Instant::now(),
            config,
            node: node_manager,
            context,
        }
    }

    async fn make_node_manager(
        ctx: Arc<Context>,
        cli_state: &CliState,
    ) -> ockam::Result<Arc<InMemoryNode>> {
        let tcp = TcpTransport::create(&ctx).await?;
        let options = TcpListenerOptions::new();
        let listener = tcp.listen(&"127.0.0.1:0", options).await?;

        let _ = cli_state
            .start_node_with_optional_values(NODE_NAME, &None, &None, Some(&listener))
            .await?;

        let trust_options = cli_state
            .retrieve_trust_options(&None, &None, &None, &None)
            .await?;

        let node_manager = Arc::new(
            InMemoryNode::new(
                &ctx,
                NodeManagerGeneralOptions::new(
                    cli_state.clone(),
                    NODE_NAME.to_string(),
                    true,
                    None,
                    false,
                ),
                NodeManagerTransportOptions::new(listener.flow_control_id().clone(), tcp),
                trust_options,
            )
            .await?,
        );

        let node_manager_worker = NodeManagerWorker::new(node_manager.clone());
        ctx.start_worker(NODEMANAGER_ADDR, node_manager_worker)
            .await?;

        ctx.flow_controls()
            .add_consumer(NODEMANAGER_ADDR, listener.flow_control_id());
        Ok(node_manager)
    }

    async fn execute_delta(&self) {
        let elapsed = self.begin.elapsed().as_secs_f32();

        self.create_relays(self.config.calculate_relays(elapsed))
            .await;

        self.create_portals(self.config.calculate_portals(elapsed))
            .await;
    }

    async fn create_relays(&self, count: usize) {
        let existing_relays = self.relays.lock().unwrap().len();
        let mut new_relays = count - existing_relays;

        while new_relays > 0 {
            let batch_size = cmp::min(new_relays, 100);

            let mut join_set = JoinSet::new();
            for _ in 0..batch_size {
                let node = self.node.clone();
                let project_addr = self.config.project_addr();
                let context = self.context.clone();
                join_set.spawn(async move {
                    let id = Self::random_id();
                    node.create_relay(&context, &project_addr, id.clone(), false, None, Some(id))
                        .await
                });
            }

            while let Some(result) = join_set.join_next().await {
                let result = result.expect("cannot join next future");
                match result {
                    Ok(info) => {
                        self.relays.lock().unwrap().insert(
                            info.alias().to_string(),
                            Relay {
                                failures_detected: 0,
                                usages: 0,
                            },
                        );
                    }
                    Err(_err) => {
                        self.relay_creation_failed
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }

            new_relays -= batch_size;
        }
    }

    fn random_id() -> String {
        format!("{:x}", rand::random::<u64>())
    }

    /// Returns the relay with the least amount of usage
    fn select_relay_and_increment_usage(&self) -> Option<String> {
        let mut guard = self.relays.lock().unwrap();
        let relay_info = guard.iter_mut().reduce(|acc, relay| {
            if acc.1.usages < relay.1.usages {
                acc
            } else {
                relay
            }
        });

        if let Some((id, relay)) = relay_info {
            relay.usages += 1;
            Some(id.clone())
        } else {
            None
        }
    }

    async fn create_portals(&self, count: usize) {
        let existing_portals = self.portals.lock().unwrap().len();
        let new_portals = count - existing_portals;

        if new_portals == 0 {
            return;
        }

        let relays = self.node.get_relays().await;

        let mut join_set = JoinSet::new();
        for _ in 0..new_portals {
            if let Some(relay_address_id) = self.select_relay_and_increment_usage() {
                let node = self.node.clone();
                let project_addr = self.config.project_addr();
                let context = self.context.clone();
                let throughput = self.config.throughput();

                let relay_flow_control_id = relays
                    .iter()
                    .find(|relay| relay.alias() == relay_address_id)
                    .unwrap()
                    .flow_control_id()
                    .clone()
                    .unwrap();

                join_set.spawn(async move {
                    let id = Self::random_id();
                    portal_simulator::create(
                        context,
                        node,
                        id.clone(),
                        project_addr,
                        Address::from_string(format!("forward_to_{relay_address_id}")),
                        throughput,
                        relay_flow_control_id,
                    )
                    .await
                    .map(|stats| (id, stats))
                });
            } else {
                println!("No relays available to create a portal, skipping.");
            }
        }

        while let Some(result) = join_set.join_next().await {
            let result = result.expect("cannot join next future");
            match result {
                Ok((id, portal_stats)) => {
                    self.portals
                        .lock()
                        .unwrap()
                        .insert(id.clone(), portal_stats);
                }
                Err(_err) => {
                    self.portal_creation_failed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }

    // measure the last second speed based on delta between previous and current values
    // and stores last 10 values it in the speed_stats
    fn measure_speed(&self) {
        let delta_sent;
        let delta_received;
        {
            let portals = self.portals.lock().unwrap();
            let total_bytes_sent: u64 = portals
                .values()
                .map(|stats| stats.bytes_sent.load(std::sync::atomic::Ordering::Relaxed))
                .sum();
            let total_bytes_received: u64 = portals
                .values()
                .map(|stats| {
                    stats
                        .bytes_received
                        .load(std::sync::atomic::Ordering::Relaxed)
                })
                .sum();

            // assume it's invoked once per second
            delta_sent = total_bytes_sent
                - self
                    .previous_bytes_sent
                    .load(std::sync::atomic::Ordering::Relaxed);
            delta_received = total_bytes_received
                - self
                    .previous_bytes_received
                    .load(std::sync::atomic::Ordering::Relaxed);

            self.previous_bytes_sent
                .store(total_bytes_sent, std::sync::atomic::Ordering::Relaxed);
            self.previous_bytes_received
                .store(total_bytes_received, std::sync::atomic::Ordering::Relaxed);
        }

        let mut guard = self.speed_stats.lock().unwrap();
        guard.push((delta_sent, delta_received));
        if guard.len() > 10 {
            guard.remove(0);
        }
    }

    fn calculate_average_speed(&self) -> (f64, f64) {
        let guard = self.speed_stats.lock().unwrap();
        let (sent, received) = guard.iter().fold((0, 0), |acc, (sent, received)| {
            (acc.0 + sent, acc.1 + received)
        });
        let size = guard.len();
        if size > 0 {
            (sent as f64 / size as f64, received as f64 / size as f64)
        } else {
            (0.0, 0.0)
        }
    }

    fn print_summary(&self) {
        let elapsed = self.begin.elapsed().as_secs();
        // print the header every 10 seconds
        if elapsed % 10 == 0 {
            println!("|  Elapsed  | Portals | Relays | M. sent | M. recv | In-fly |  B. sent  |  B. recv  | Spe. sent  | Spe. recv  | M. OOO | Errors |")
        }

        let (speed_sent, speed_received) = self.calculate_average_speed();

        let errors: u32 = {
            let relays = self.relays.lock().unwrap();
            relays
                .values()
                .map(|relay| relay.failures_detected)
                .sum::<u32>()
        } + self
            .relay_creation_failed
            .load(std::sync::atomic::Ordering::Relaxed)
            + self
                .portal_creation_failed
                .load(std::sync::atomic::Ordering::Relaxed);

        {
            let portals = self.portals.lock().unwrap();
            let total_messages_sent: u64 = portals
                .values()
                .map(|stats| {
                    stats
                        .messages_sent
                        .load(std::sync::atomic::Ordering::Relaxed)
                })
                .sum();
            let total_bytes_sent: u64 = portals
                .values()
                .map(|stats| stats.bytes_sent.load(std::sync::atomic::Ordering::Relaxed))
                .sum();
            let total_messages_received: u64 = portals
                .values()
                .map(|stats| {
                    stats
                        .messages_received
                        .load(std::sync::atomic::Ordering::Relaxed)
                })
                .sum();
            let messages_out_of_order: u64 = portals
                .values()
                .map(|stats| {
                    stats
                        .messages_out_of_order
                        .load(std::sync::atomic::Ordering::Relaxed)
                })
                .sum();
            let total_bytes_received: u64 = portals
                .values()
                .map(|stats| {
                    stats
                        .bytes_received
                        .load(std::sync::atomic::Ordering::Relaxed)
                })
                .sum();

            let total_portals = portals.len();
            let total_relays = self.relays.lock().unwrap().len();
            let in_fly = total_messages_sent - total_messages_received;

            println!(
                "| {:^9} | {:^7} | {:^6} | {:^7} | {:^7} | {:^6} | {:^9} | {:^9} | {:^10} | {:^10} | {:^6} | {:^6} |",
                time_to_human_readable(elapsed),
                total_portals,
                total_relays,
                total_messages_sent,
                total_messages_received,
                in_fly,
                bytes_to_human_readable(total_bytes_sent),
                bytes_to_human_readable(total_bytes_received),
                speed_to_human_readable(speed_sent),
                speed_to_human_readable(speed_received),
                messages_out_of_order,
                errors,
            );
        }
    }
}

fn time_to_human_readable(elapsed: u64) -> String {
    let hours = elapsed / 3600;
    let minutes = (elapsed % 3600) / 60;
    let seconds = elapsed % 60;

    if hours > 0 {
        format!("{hours}h{minutes}m{seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds:02}s")
    }
}

fn bytes_to_human_readable(bytes: u64) -> String {
    let bytes = bytes as f64;
    let kb = bytes / 1024.0;
    let mb = kb / 1024.0;
    let gb = mb / 1024.0;
    if gb > 0.1 {
        format!("{:.2} GB", gb)
    } else if mb > 0.1 {
        format!("{:.2} MB", mb)
    } else if kb > 0.1 {
        format!("{:.2} KB", kb)
    } else {
        format!("{} B", bytes)
    }
}

fn speed_to_human_readable(bytes: f64) -> String {
    let bits = bytes * 8.0;
    let kb = bits / 1000.0;
    let mb = kb / 1000.0;
    let gb = mb / 1000.0;
    if gb >= 0.1 {
        format!("{:.2} Gbps", gb)
    } else if mb >= 0.1 {
        format!("{:.2} Mbps", mb)
    } else if kb >= 0.1 {
        format!("{:.2} Kbps", kb)
    } else {
        format!("{:.2} bps", bits)
    }
}
