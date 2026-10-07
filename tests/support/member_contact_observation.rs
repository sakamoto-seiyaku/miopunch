//! Read-only observation around the real member transport; all decisions delegate.
use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use miopunch_native_probe::member_transport::MemberSignalingTransport;
use std::{
    io,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Instant,
};

#[derive(Clone, Debug, Default)]
pub struct Events(Arc<Mutex<(Option<Instant>, Vec<serde_json::Value>)>>);
impl Events {
    pub fn begin(&self) {
        *self.0.lock().unwrap() = (Some(Instant::now()), vec![]);
    }
    pub fn record(&self, kind: &str, bytes: usize, outcome: &str) {
        let mut state = self.0.lock().unwrap();
        if let Some(start) = state.0 {
            state.1.push(serde_json::json!({"kind":kind,"bytes":bytes,
                "outcome":outcome,"elapsedUs":start.elapsed().as_micros()}));
        }
    }
    pub fn snapshot(&self) -> Vec<serde_json::Value> {
        self.0.lock().unwrap().1.clone()
    }
}

#[derive(Debug)]
pub struct ObservedMember {
    pub real: Arc<MemberSignalingTransport>,
    pub events: Events,
}
impl CustomTransport for ObservedMember {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        Ok(Box::new(ObservedEndpoint {
            real: self.real.bind()?,
            events: self.events.clone(),
        }))
    }
}
#[derive(Debug)]
struct ObservedEndpoint {
    real: Box<dyn CustomEndpoint>,
    events: Events,
}
impl CustomEndpoint for ObservedEndpoint {
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.real.watch_local_addrs()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(ObservedSender {
            real: self.real.create_sender(),
            events: self.events.clone(),
        })
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        self.real.poll_recv(cx, bufs, metas, infos)
    }
    fn max_transmit_segments(&self) -> NonZeroUsize {
        self.real.max_transmit_segments()
    }
}
#[derive(Debug)]
struct ObservedSender {
    real: Arc<dyn CustomSender>,
    events: Events,
}
impl CustomSender for ObservedSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        self.real.is_valid_send_addr(addr)
    }
    fn can_bootstrap_connection(&self, addr: &CustomAddr) -> bool {
        self.real.can_bootstrap_connection(addr)
    }
    fn is_valid_connection_addr(&self, addr: &CustomAddr, id: usize) -> bool {
        self.real.is_valid_connection_addr(addr, id)
    }
    fn guards_application_data(&self, addr: &CustomAddr) -> bool {
        self.real.guards_application_data(addr)
    }
    fn is_data_path_qualified(&self, addr: &CustomAddr, id: usize) -> bool {
        self.real.is_data_path_qualified(addr, id)
    }
    fn poll_send(
        &self,
        cx: &mut Context<'_>,
        dst: &CustomAddr,
        src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let result = self.real.poll_send(cx, dst, src, transmit);
        let outcome = match &result {
            Poll::Pending => "pending".to_owned(),
            Poll::Ready(Ok(())) => "accepted".to_owned(),
            Poll::Ready(Err(error)) => format!("{:?}", error.kind()),
        };
        self.events
            .record("memberSend", transmit.contents.len(), &outcome);
        result
    }
}
