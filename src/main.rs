use std::sync::Arc;

use ahash::AHashMap;
use parking_lot::RwLock;
use tokio::sync::oneshot::Receiver;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use twilight_gateway::Shard;
use twilight_http::{request::channel::reaction::RequestReactionType, Client as DiscordClient};
use twilight_model::{
    channel::ChannelType,
    gateway::{event::Event, payload::incoming::ThreadCreate, CloseFrame, Intents, ShardId},
    id::{marker::ChannelMarker, Id},
};

#[macro_use]
extern crate tracing;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let token =
        std::env::var("DISCORD_TOKEN").expect("Failed to get DISCORD_TOKEN environment variable");
    let intents = Intents::GUILDS;
    let shard = Shard::new(ShardId::ONE, token.clone(), intents);
    info!("created shard");
    let client = DiscordClient::new(token);
    let forums = RwLock::new(AHashMap::with_capacity(256));
    let state = Arc::new(InnerAppState { client, forums });
    let (shutdown_s, shutdown_r) = tokio::sync::oneshot::channel();
    debug!("registering shutdown handler");
    #[cfg(not(unix))]
    compile_error!("This application only supports Unix platforms. Consider WSL or docker.");
    tokio::spawn(async move {
        let mut sig =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        let ctrlc = tokio::signal::ctrl_c();
        tokio::select! {
            _v = sig.recv() => {},
            _v = ctrlc => {}
        }
        info!("Shutting down!");
        shutdown_s
            .send(())
            .expect("Failed to shut down, is the shutdown handler running?");
    });
    event_loop(&state, shard, shutdown_r).await;
}

async fn event_loop(state: &AppState, mut shard: Shard, mut shutdown_r: Receiver<()>) {
    loop {
        #[allow(clippy::redundant_pub_crate)]
        let next = tokio::select! {
            v = shard.next_event() => v,
            _ = &mut shutdown_r => break,
        };
        trace!(?next, "got new event");
        let event = match next {
            Ok(event) => event,
            Err(source) => {
                error!(?source, "error receiving event");
                if source.is_fatal() {
                    break;
                }
                continue;
            }
        };
        if let Event::ThreadCreate(thread) = event {
            wrap_result(on_thread_create(state, thread).await)
        }
    }
    let _ = shard.close(CloseFrame::NORMAL).await;
}

fn wrap_result<T>(result: Result<T, Error>) {
    if let Err(source) = result {
        error!(?source, "encountered an error");
    }
}

async fn on_thread_create(state: &AppState, thread: Box<ThreadCreate>) -> Result<(), Error> {
    let parent = thread.parent_id.ok_or(Error::NoThreadParentId)?;
    if !is_forum_post(state, parent).await? {
        debug!(
            parent = parent.get(),
            thread = thread.id.get(),
            "Skipping channel because parent was not a forum"
        );
        return Ok(());
    }
    state
        .client
        .create_reaction(
            thread.id,
            thread.id.cast(),
            &RequestReactionType::Unicode { name: "⬆️" },
        )
        .await?;
    Ok(())
}

async fn is_forum_post(state: &AppState, parent: Id<ChannelMarker>) -> Result<bool, Error> {
    if let Some(kind) = state.forums.read().get(&parent) {
        return Ok(*kind);
    }
    let channel_kind = state.client.channel(parent).await?.model().await?.kind;
    Ok(matches!(channel_kind, ChannelType::GuildForum))
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("twilight-http error: {0}")]
    DiscordApi(#[from] twilight_http::Error),
    #[error("twilight-http deserializer error: {0}")]
    BodyDeserialize(#[from] twilight_http::response::DeserializeBodyError),
    #[error("Discord did not send a parent channel ID, are you sure this is a thread?")]
    NoThreadParentId,
}

pub struct InnerAppState {
    client: DiscordClient,
    forums: RwLock<AHashMap<Id<ChannelMarker>, bool>>,
}

pub type AppState = Arc<InnerAppState>;
