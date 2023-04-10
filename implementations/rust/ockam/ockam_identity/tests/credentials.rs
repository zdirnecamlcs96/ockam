use ockam_core::compat::{boxed::Box, sync::Arc};
use ockam_core::{async_trait, AllowAll, Any, DenyAll, Mailboxes};
use ockam_core::{route, Result, Routed, Worker};
use ockam_identity::authenticated_storage::{
    mem::InMemoryStorage, AuthenticatedAttributeStorage, IdentityAttributeStorage,
    IdentityAttributeStorageReader,
};
use ockam_identity::credential::access_control::CredentialAccessControl;
use ockam_identity::credential::{Credential, CredentialExchangeMode};
use ockam_identity::{
    AuthorityInfo, CredentialMemoryRetriever, Identity, SecureChannelListenerOptions,
    SecureChannelOptions, TrustContext, TrustIdentifierPolicy,
};

use ockam_node::{Context, MessageSendReceiveOptions, WorkerBuilder};
use ockam_vault::Vault;
use std::sync::atomic::{AtomicI8, Ordering};
use std::time::Duration;

#[ockam_macros::test]
async fn full_flow_oneway(ctx: &mut Context) -> Result<()> {
    let vault = Vault::create();

    let authority = Identity::create(ctx, vault.clone()).await?;
    let server = Identity::create(ctx, vault.clone()).await?;

    server
        .create_secure_channel_listener("listener", SecureChannelListenerOptions::new())
        .await?;

    let trust_context = TrustContext::new(
        "test_trust_context_id".to_string(),
        Some(AuthorityInfo::new(authority.to_public().await?, None)),
    );
    server
        .create_secure_channel_listener(
            "listener",
            SecureChannelListenerOptions::new().with_trust_context(trust_context.clone()),
        )
        .await?;

    let client = Identity::create(ctx, vault).await?;
    let _channel = client
        .create_secure_channel(
            route!["listener"],
            SecureChannelOptions::new()
                .with_trust_policy(TrustIdentifierPolicy::new(server.identifier().clone()))
                .with_trust_context(trust_context),
        )
        .await?;

    let credential_builder = Credential::builder(client.identifier().clone());
    let credential = credential_builder.with_attribute("is_superuser", b"true");
    let credential = authority.issue_credential(credential).await?;

    client
        .create_secure_channel(
            route!["listener"],
            SecureChannelOptions::new()
                .with_trust_policy(TrustIdentifierPolicy::new(server.identifier().clone()))
                .with_credential(credential)
                .with_credential_exchange_mode(CredentialExchangeMode::Oneway),
        )
        .await?;

    let attrs = AuthenticatedAttributeStorage::new(server.authenticated_storage())
        .get_attributes(client.identifier())
        .await?
        .unwrap();

    let val = attrs.attrs().get("is_superuser").unwrap();

    assert_eq!(val.as_slice(), b"true");

    ctx.stop().await
}

#[ockam_macros::test]
async fn full_flow_twoway(ctx: &mut Context) -> Result<()> {
    let vault = Vault::create();

    let authority = Identity::create(ctx, vault.clone()).await?;
    let client1 = Identity::create(ctx, vault.clone()).await?;
    let client2 = Identity::create(ctx, vault).await?;

    let credential2 =
        Credential::builder(client2.identifier().clone()).with_attribute("is_admin", b"true");

    let credential2 = authority.issue_credential(credential2).await?;

    let trust_context = TrustContext::new(
        "test_trust_context_id".to_string(),
        Some(AuthorityInfo::new(
            authority.to_public().await?,
            Some(Arc::new(CredentialMemoryRetriever::new(credential2))),
        )),
    );
    client2
        .create_secure_channel_listener(
            "listener",
            SecureChannelListenerOptions::new().with_trust_context(tru),
        )
        .await?;

    let credential1 =
        Credential::builder(client1.identifier().clone()).with_attribute("is_user", b"true");

    let credential1 = authority.issue_credential(credential1).await?;

    client1
        .create_secure_channel(
            route!["listener"],
            SecureChannelOptions::new()
                .with_trust_context(trust_context)
                .with_credential(credential1)
                .with_credential_exchange_mode(CredentialExchangeMode::Mutual),
        )
        .await?;

    let attrs1 = AuthenticatedAttributeStorage::new(client2.authenticated_storage())
        .get_attributes(client1.identifier())
        .await?
        .unwrap();

    assert_eq!(attrs1.attrs().get("is_user").unwrap().as_slice(), b"true");

    let attrs2 = AuthenticatedAttributeStorage::new(client1.authenticated_storage())
        .get_attributes(client2.identifier())
        .await?
        .unwrap();

    assert_eq!(attrs2.attrs().get("is_admin").unwrap().as_slice(), b"true");

    ctx.stop().await
}

struct CountingWorker {
    msgs_count: Arc<AtomicI8>,
}

#[async_trait]
impl Worker for CountingWorker {
    type Context = Context;
    type Message = Any;

    async fn handle_message(
        &mut self,
        _context: &mut Self::Context,
        _msg: Routed<Self::Message>,
    ) -> Result<()> {
        let _ = self.msgs_count.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }
}

#[ockam_macros::test]
async fn access_control(ctx: &mut Context) -> Result<()> {
    let vault = Vault::create();
    let authority = Identity::create(ctx, vault.clone()).await?;
    let server = Identity::create(ctx, vault.clone()).await?;

    let trust_context = TrustContext::new(
        "test_trust_context_id".to_string(),
        Some(AuthorityInfo::new(authority.to_public().await?, None)),
    );

    server
        .create_secure_channel_listener(
            "listener",
            SecureChannelListenerOptions::new().with_trust_context(trust_context.clone()),
        )
        .await?;

    let client = Identity::create(ctx, vault.clone()).await?;
    let _channel = client
        .create_secure_channel(
            route!["listener"],
            SecureChannelOptions::new()
                .with_trust_context(trust_context.clone())
                .with_trust_policy(TrustIdentifierPolicy::new(server.identifier().clone())),
        )
        .await?;

    let credential_builder = Credential::builder(client.identifier().clone());
    let credential = credential_builder.with_attribute("is_superuser", b"true");
    let credential = authority.issue_credential(credential).await?;

    let counter = Arc::new(AtomicI8::new(0));

    let worker = CountingWorker {
        msgs_count: counter.clone(),
    };

    let required_attributes = vec![("is_superuser".to_string(), b"true".to_vec())];
    let access_control = CredentialAccessControl::new(
        &required_attributes,
        AuthenticatedAttributeStorage::new(server.authenticated_storage()),
    );

    WorkerBuilder::with_access_control(
        Arc::new(access_control),
        Arc::new(DenyAll),
        "counter",
        worker,
    )
    .start(ctx)
    .await?;

    ctx.wait_for("counter").await?;
    assert_eq!(counter.load(Ordering::Relaxed), 0);

    // first secure channel without credential will be rejected
    let channel = client
        .create_secure_channel(
            route!["listener"],
            SecureChannelOptions::new()
                .with_trust_context(trust_context.clone())
                .with_trust_policy(TrustIdentifierPolicy::new(server.identifier().clone())),
        )
        .await?;

    let child_ctx = ctx
        .new_detached_with_mailboxes(Mailboxes::main(
            "child",
            Arc::new(AllowAll),
            Arc::new(AllowAll),
        ))
        .await?;

    child_ctx
        .send(route![channel.clone(), "counter"], "Hello".to_string())
        .await?;
    ctx.sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 0);

    // second secure channel with credential will pass
    let channel = client
        .create_secure_channel(
            route!["listener"],
            SecureChannelOptions::new()
                .with_credential(credential)
                .with_trust_context(trust_context)
                .with_trust_policy(TrustIdentifierPolicy::new(server.identifier().clone())),
        )
        .await?;

    child_ctx
        .send(route![channel, "counter"], "Hello".to_string())
        .await?;
    ctx.sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 1);

    ctx.stop().await
}
