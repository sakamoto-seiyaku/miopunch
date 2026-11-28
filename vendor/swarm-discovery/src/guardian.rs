use crate::{
    receiver::{interface_receiver, receiver, ReceiverError},
    sender::{self, sender},
    socket::Sockets,
    updater::updater,
    Discoverer,
};
use acto::{AcTokioRuntime, ActoCell, ActoId, ActoInput, ActoRef};
use hickory_proto::rr::Name;
use std::{
    collections::{HashMap, HashSet},
    mem::replace,
    net::{IpAddr, Ipv4Addr},
};

pub enum Input {
    Ipv4Interfaces(Vec<Ipv4Addr>),
    Ipv6Interfaces(Vec<u32>),
    RemoveAll,
    RemovePort(u16),
    RemoveAddr(IpAddr),
    AddAddr(u16, Vec<IpAddr>),
    SetTxt(String, Option<String>),
    RemoveTxt(String),
    AddInterface(IpAddr),
    RemoveInterface(IpAddr),
}

pub async fn guardian(
    mut ctx: ActoCell<Input, AcTokioRuntime, Result<(), ReceiverError>>,
    mut discoverer: Discoverer,
    sockets: Sockets,
    service_name: Name,
) {
    let callback = replace(&mut discoverer.callback, Box::new(|_, _| {}));
    let tau = discoverer.tau;
    let phi = discoverer.phi;
    let upd_ref = ctx.supervise(
        ctx.spawn("updater", move |ctx| updater(ctx, tau, phi, callback))
            .map_handle(Ok),
    );

    let sockets2 = sockets.clone();
    let sn = service_name.clone();
    let snd_ref = ctx.supervise(
        ctx.spawn("sender", move |ctx| {
            sender(ctx, sockets, upd_ref, discoverer, sn)
        })
        .map_handle(Ok),
    );

    if let Some(v4) = sockets2.v4() {
        let service_name = service_name.clone();
        let snd_ref = snd_ref.clone();
        ctx.spawn_supervised("receiver_v4", move |ctx| {
            receiver(ctx, service_name, v4, snd_ref)
        });
    }

    if let Some(v6) = sockets2.v6() {
        let service_name = service_name.clone();
        let snd_ref = snd_ref.clone();
        ctx.spawn_supervised("receiver_v6", move |ctx| {
            receiver(ctx, service_name, v6, snd_ref)
        });
    }

    // Track interface receivers so we can stop them when interfaces are removed
    let mut interface_receivers: HashMap<Ipv4Addr, ActoRef<()>> = HashMap::new();
    let mut retired_receivers = HashSet::new();
    sync_interface_receivers(
        &mut ctx,
        &sockets2,
        &service_name,
        &snd_ref,
        &mut interface_receivers,
        &mut retired_receivers,
    );

    // only stop when a supervised actor stops
    loop {
        let msg = ctx.recv().await;
        match msg {
            ActoInput::NoMoreSenders => {}
            ActoInput::Supervision { id, name, result } => {
                if retired_receivers.remove(&id) {
                    tracing::debug!("retired interface receiver {:?} ({}) stopped", id, name);
                    continue;
                }
                match result {
                    Ok(Ok(_)) => tracing::warn!("actor {:?} ({}) stopped", id, name),
                    Ok(Err(e)) => {
                        tracing::warn!("actor {:?} ({}) failed: {}", id, name, e)
                    }
                    Err(e) => {
                        tracing::warn!("actor {:?} ({}) aborted: {}", id, name, e);
                    }
                }
                break;
            }
            ActoInput::Message(msg) => match &msg {
                Input::Ipv4Interfaces(interfaces) => {
                    for previous in sockets2.get_all_interface_addresses_v4() {
                        if !interfaces.contains(&previous) {
                            sockets2.remove_interface_v4(previous);
                        }
                    }
                    // Compare with successfully created sockets, not the last
                    // requested snapshot, so a failed addition can be retried.
                    for addr in interfaces {
                        if let Err(error) = sockets2.add_interface_v4(*addr) {
                            tracing::warn!("Failed to add interface {}: {}", addr, error);
                        }
                    }
                    sync_interface_receivers(
                        &mut ctx,
                        &sockets2,
                        &service_name,
                        &snd_ref,
                        &mut interface_receivers,
                        &mut retired_receivers,
                    );
                }
                Input::Ipv6Interfaces(interfaces) => sockets2.set_ipv6_interfaces(interfaces),
                Input::AddInterface(addr) => {
                    if let IpAddr::V4(ipv4) = addr {
                        if let Err(e) = sockets2.add_interface_v4(*ipv4) {
                            tracing::warn!("Failed to add interface {}: {}", addr, e);
                        }
                        sync_interface_receivers(
                            &mut ctx,
                            &sockets2,
                            &service_name,
                            &snd_ref,
                            &mut interface_receivers,
                            &mut retired_receivers,
                        );
                    }
                }
                Input::RemoveInterface(addr) => {
                    if let IpAddr::V4(ipv4) = addr {
                        sockets2.remove_interface_v4(*ipv4);
                        sync_interface_receivers(
                            &mut ctx,
                            &sockets2,
                            &service_name,
                            &snd_ref,
                            &mut interface_receivers,
                            &mut retired_receivers,
                        );
                    }
                }
                _ => {
                    snd_ref.send(sender::MdnsMsg::Update(msg));
                }
            },
        }
    }
}

fn sync_interface_receivers(
    ctx: &mut ActoCell<Input, AcTokioRuntime, Result<(), ReceiverError>>,
    sockets: &Sockets,
    service_name: &Name,
    sender: &ActoRef<sender::MdnsMsg>,
    receivers: &mut HashMap<Ipv4Addr, ActoRef<()>>,
    retired: &mut HashSet<ActoId>,
) {
    let current = sockets.get_all_interface_addresses_v4();
    let removed: Vec<_> = receivers
        .keys()
        .filter(|addr| !current.contains(addr))
        .copied()
        .collect();
    for addr in removed {
        let receiver = receivers.remove(&addr).expect("known interface receiver");
        retired.insert(receiver.id());
        // interface_receiver observes its mailbox closing and releases the
        // socket. Ordinary wildcard receivers retain their original lifetime.
        drop(receiver);
    }
    for addr in current {
        receivers.entry(addr).or_insert_with(|| {
            let socket = sockets
                .get_interface_socket_v4(addr)
                .expect("registered interface socket");
            let service_name = service_name.clone();
            let sender = sender.clone();
            ctx.spawn_supervised(&format!("receiver_interface_{}", addr), move |ctx| {
                interface_receiver(ctx, service_name, socket, sender)
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acto::{AcTokio, ActoHandle, ActoRuntime};
    use std::{net::Ipv4Addr, sync::Arc, time::Duration};

    #[tokio::test]
    async fn removing_an_interface_releases_its_receiver_and_keeps_discovery_alive() {
        let sockets = Sockets::new(crate::IpClass::V4Only, vec![Ipv4Addr::LOCALHOST]).unwrap();
        let actor_sockets = sockets.clone();
        let runtime =
            AcTokio::from_handle("interface-retirement", tokio::runtime::Handle::current());
        let actor = runtime.spawn_actor("guardian", move |ctx| {
            guardian(
                ctx,
                Discoverer::new_interactive("retirement".into(), "peer".into()),
                actor_sockets,
                Name::from_ascii("_retirement._udp.local.").unwrap(),
            )
        });
        for round in 0..3 {
            let socket = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if let Some(socket) = sockets.get_interface_socket_v4(Ipv4Addr::LOCALHOST) {
                        break socket;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("healthy guardian adds the next interface");
            let old = Arc::downgrade(&socket);
            drop(socket);
            actor.me.send(if round == 0 {
                Input::RemoveInterface(Ipv4Addr::LOCALHOST.into())
            } else {
                Input::Ipv4Interfaces(vec![])
            });
            tokio::time::timeout(Duration::from_secs(1), async {
                while old.strong_count() != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("retired interface receiver releases its socket");
            assert!(
                !actor.handle.is_finished(),
                "planned retirement must not stop discovery"
            );
            actor.me.send(if round == 0 {
                Input::AddInterface(Ipv4Addr::LOCALHOST.into())
            } else {
                Input::Ipv4Interfaces(vec![Ipv4Addr::LOCALHOST])
            });
        }
        actor.handle.abort();
    }
}
