use async_trait::async_trait;

use clap::{command, Args};
use colorful::Colorful;
use tokio::{sync::Mutex, try_join};

use ockam::Context;
use ockam_abac::Expr;
use ockam_api::colors::OckamColor;
use ockam_api::nodes::models::services::StartKafkaOutletRequest;
use ockam_api::nodes::models::services::StartServiceRequest;
use ockam_api::nodes::BackgroundNodeClient;
use ockam_api::{fmt_log, fmt_ok};
use ockam_core::api::Request;

use crate::node::util::initialize_default_node;
use crate::{
    kafka::{kafka_default_outlet_addr, kafka_default_outlet_server},
    node::NodeOpts,
    service::start::start_service_impl,
    Command, CommandGlobalOpts,
};

/// Create a new Kafka Outlet
#[derive(Clone, Debug, Args)]
pub struct CreateCommand {
    #[command(flatten)]
    pub node_opts: NodeOpts,
    /// The local address of the service
    #[arg(long, default_value_t = kafka_default_outlet_addr())]
    pub addr: String,
    /// The address of the kafka bootstrap broker
    #[arg(long, default_value_t = kafka_default_outlet_server())]
    pub bootstrap_server: String,
    /// If tls is set then the outlet will establish a TLS connection over TCP
    #[arg(long, id = "BOOLEAN")]
    pub tls: bool,
    /// Policy expression that will be used for access control to the Kafka Outlet.
    /// If you don't provide it, the policy set for the "tcp-outlet" resource type will be used.
    ///
    /// You can check the fallback policy with `ockam policy show --resource-type tcp-outlet`.
    #[arg(hide = true, long = "allow", id = "EXPRESSION")]
    pub policy_expression: Option<Expr>,
}

#[async_trait]
impl Command for CreateCommand {
    const NAME: &'static str = "kafka-outlet create";

    async fn async_run(self, ctx: &Context, opts: CommandGlobalOpts) -> crate::Result<()> {
        initialize_default_node(ctx, &opts).await?;
        opts.terminal
            .write_line(&fmt_log!("Creating KafkaOutlet service"))?;
        let is_finished = Mutex::new(false);
        let send_req = async {
            let payload = StartKafkaOutletRequest::new(
                self.bootstrap_server.clone(),
                self.tls,
                self.policy_expression,
            );
            let payload = StartServiceRequest::new(payload, &self.addr);
            let req = Request::post("/node/services/kafka_outlet").body(payload);
            let node =
                BackgroundNodeClient::create(ctx, &opts.state, &self.node_opts.at_node).await?;

            start_service_impl(ctx, &node, "KafkaOutlet", req).await?;
            *is_finished.lock().await = true;

            Ok(())
        };

        let msgs = vec![
            format!(
                "Building KafkaOutlet service {}",
                &self
                    .addr
                    .to_string()
                    .color(OckamColor::PrimaryResource.color())
            ),
            format!(
                "Starting KafkaOutlet service, connecting to {}",
                &self
                    .bootstrap_server
                    .to_string()
                    .color(OckamColor::PrimaryResource.color())
            ),
        ];
        let progress_output = opts.terminal.progress_output(&msgs, &is_finished);
        let (_, _) = try_join!(send_req, progress_output)?;

        opts.terminal
            .stdout()
            .plain(fmt_ok!(
                "KafkaOutlet service started at {}\n",
                &self
                    .bootstrap_server
                    .to_string()
                    .color(OckamColor::PrimaryResource.color())
            ))
            .write_line()?;

        Ok(())
    }
}
