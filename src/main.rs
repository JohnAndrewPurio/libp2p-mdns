use std::{
    error::Error,
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration
};

use futures::StreamExt;
use libp2p::{
    gossipsub, mdns, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, Swarm,
};
use tokio::{io, io::AsyncBufReadExt, select};
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let mut swarm = configure_swarm()?;

    let topic = gossipsub::IdentTopic::new("test-net");

    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

    // Tell the swarm to listen on all interfaces and a random, OS-assigned port.
    swarm.listen_on("/ip6/::/udp/0/quic-v1".parse()?)?;

    // Dial the peer identified by the multi-address given as the second
    // command-line argument, if any
    if let Some(addr) = std::env::args().nth(1) {
        let remote: Multiaddr = addr.parse()?;
        swarm.dial(remote)?;
        println!("Dialed {addr}");
    }

    let mut stdin = io::BufReader::new(io::stdin()).lines();

    loop {
        select! {
            Ok(Some(line)) = stdin.next_line() => {
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), line.as_bytes()) {
                    println!("Publish error: {e:?}");
                }
            }
            event = swarm.select_next_some() => {
                let behaviour = swarm.behaviour_mut();

                let _ = handle_event(event, behaviour).await;
            }
        }
    }
}

fn configure_swarm() -> Result<Swarm<Behaviour>, Box<dyn Error>> {
    let swarm = libp2p::SwarmBuilder::with_new_identity()
    .with_tokio()
    .with_quic()
    .with_behaviour(|key| {
        let message_id_fn = |message: &gossipsub::Message| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);

            gossipsub::MessageId::from(s.finish().to_string())
        };

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10))
            .validation_mode(gossipsub::ValidationMode::Strict)
            .message_id_fn(message_id_fn)
            .build()
            .map_err(|msg| io::Error::new(io::ErrorKind::Other, msg))?;

        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(key.clone()),
            gossipsub_config,
        )?;

        let mdns =
            mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;

        Ok(Behaviour {
            gossipsub,
            mdns,
            ping: ping::Behaviour::default(),
        })
    })?
    .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
    .build();

    Ok(swarm)
}

async fn handle_event(
    event: SwarmEvent<BehaviourEvent>,
    behaviour: &mut Behaviour,
) -> Result<(), Box<dyn Error>> {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            println!("Local node is listening on {address:?}")
        }

        SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event {
            peer,
            result: Ok(result),
            ..
        })) => println!("Pinged {peer:?} with latency of {result:?}"),

        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
            for (peer_id, _multiaddr) in list {
                println!("mDNS discovered a new peer: {peer_id}");

                behaviour.gossipsub.add_explicit_peer(&peer_id);
            }
        }

        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
            for (peer_id, _multiaddr) in list {
                println!("mDNS discover peer has expired: {peer_id}");

                behaviour.gossipsub.remove_explicit_peer(&peer_id);
            }
        }

        SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message_id,
            message,
        })) => println!(
            "Got message: '{}' with id: {message_id} from peer: {propagation_source}",
            String::from_utf8_lossy(&message.data)
        ),

        _ => {}
    };

    Ok(())
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
    ping: ping::Behaviour,
}
